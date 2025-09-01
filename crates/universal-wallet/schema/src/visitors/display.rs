use std::collections::BTreeMap;
use std::fmt::Write;

use thiserror::Error;

use crate::schema::Primitive;
use crate::ty::visitor::{ResolutionError, TypeResolver, TypeVisitor};
use crate::ty::{
    ByteDisplay, Enum, FixedPointDisplay, IntegerDisplay, IntegerType, LinkingScheme, Struct, Tuple,
};

type Delimiters = (&'static str, &'static str);

pub type Result<T, E = FormatError> = core::result::Result<T, E>;

/// The largest input size to display (in bytes)
pub const MAX_INPUT_CHUNK: usize = 65;

#[derive(Debug, Error, Clone)]
pub enum FormatError {
    #[error("Core error: {0}")]
    Core(#[from] core::fmt::Error),
    #[error("The input could not be displayed as bech32: {0}")]
    InvalidBech32(#[from] bech32::EncodeError),
    #[error("The input is not a valid utf-8 string: {0}")]
    InvalidString(#[from] core::str::Utf8Error),
    #[error("Invalid discriminant `{discriminant}` for {type_name}")]
    InvalidDiscriminant { type_name: String, discriminant: u8 },
    #[error(transparent)]
    UnresolvedType(#[from] ResolutionError),
    #[error("A discriminant is required for items of type `{type_name}` but the input ended without providing one.")]
    MissingDiscriminant { type_name: String },
    #[error("The input claimed to provide an integer {claimed_size} bytes wide, but the maximum allowed size is 16 bytes.")]
    IntegerTooLarge { claimed_size: u8 },
    #[error("The input claimed to provide an integer {claimed_size} bytes wide, but only provided {bytes_available} additional bytes of input.")]
    MissingIntegerInput {
        claimed_size: u8,
        bytes_available: u8,
    },
    #[error("The input claimed to provide a byte array {claimed_size} bytes wide, but only provided {bytes_available} additional bytes of input.")]
    MissingBytesInput {
        claimed_size: usize,
        bytes_available: usize,
    },
    #[error("The input's attributes reference a sibling field with a byte offset, but offset was outside the range of the field's value.")]
    FieldReferenceOffsetOutOfBounds(usize),
    #[error("Fixed-point integer formatting specified more decimals than is plausible for an integer: {0}")]
    TooManyDecimalsForInt(u8),
    #[error("The input should have contained a vector but did not provide one.")]
    MissingVecLength,
    #[error("The input should have contained a string but did not provide one.")]
    MissingStringLength,
    #[error("The provided input had leftover bytes that weren't displayed.")]
    UnusedInput,
    #[error(
        "A structs's display template did not provide sufficient slots to display its fields."
    )]
    InsufficientTemplateSlots,
    #[error("A struct's display template had more slots than the struct had fields.")]
    UnusedTemplateSlots,
}

pub struct Output<'a, W> {
    f: &'a mut W,
    silent: bool,
    // Incremented every time a peek starts, decremented when it ends.
    // We are peeking IFF `peeking > 0`.
    peeking: u32,
}

impl<'a, W> Output<'a, W> {
    pub fn new(f: &'a mut W) -> Self {
        Self {
            f,
            silent: false,
            peeking: 0,
        }
    }

    pub fn start_peek(&mut self) {
        self.peeking = self.peeking.checked_add(1).unwrap();
    }

    pub fn end_peek(&mut self) {
        self.peeking = self
            .peeking
            .checked_sub(1)
            .expect("Underflow when ending peek. This is a bug in the schema display logic.")
    }

    pub fn peeking(&self) -> bool {
        self.peeking > 0
    }
}

impl<W: core::fmt::Write> core::fmt::Write for Output<'_, W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if self.peeking() {
            return Ok(());
        }
        if self.silent {
            return Ok(());
        }
        self.f.write_str(s)
    }
}

#[allow(unused)]
pub struct Input<'a> {
    buf: &'a mut &'a [u8],
    peeking: bool,
    /// A stack of origins for nested peek processing, to calculate relative field offsets. First
    /// element is always 0 for the outermost peek.
    peek_origins_stack: Vec<usize>,
    /// The current point we're reading at, while peeking, from the absolute start of the buffer
    /// (the current start, i.e. from the point we started peeking).
    peek_cursor: usize,
}

impl<'a> Input<'a> {
    pub fn new(buf: &'a mut &'a [u8]) -> Self {
        Self {
            buf,
            peeking: false,
            peek_origins_stack: Vec::new(),
            peek_cursor: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    fn check_remaining_bytes(&self, mut len: usize) -> Result<(), FormatError> {
        if self.peeking {
            len += self.peek_cursor;
        }
        if self.buf.len() < len {
            return Err(FormatError::MissingBytesInput {
                claimed_size: len,
                bytes_available: self.buf.len(),
            });
        }
        Ok(())
    }

    /// Splits the first `len` bytes from the input, returning them as a slice and updating the input buffer.
    /// Returns an error if there are not enough bytes remaining to fulfill the request.
    pub fn advance(&mut self, len: usize) -> Result<&[u8], FormatError> {
        self.check_remaining_bytes(len)?;
        if self.peeking {
            let end_cursor = self.peek_cursor + len;
            let slice = &self.buf[self.peek_cursor..end_cursor];
            self.peek_cursor = end_cursor;
            Ok(slice)
        } else {
            let (leading, rest) = self.buf.split_at(len);
            *self.buf = rest;
            Ok(leading)
        }
    }

    /// Similar to `advance()`, but does not advance the input, only returns a reference to the next `len` bytes.
    ///
    /// Not to be confused with the struct-wide peeking functionality that switches between
    /// consuming the buffer and advancing a cursor.
    pub fn local_peek_bytes(&self, len: usize) -> Result<&[u8], FormatError> {
        self.check_remaining_bytes(len)?;
        // When not in a global peek, peek_cursor == 0 so this is a no-op
        let end = self.peek_cursor + len;
        Ok(&self.buf[self.peek_cursor..end])
    }

    /// Similar to `local_peek_bytes()`, but returns a single byte.
    ///
    /// Not to be confused with the struct-wide peeking functionality that switches between
    /// consuming the buffer and advancing a cursor.
    pub fn local_peek_byte(&self, offset: usize) -> Result<u8, FormatError> {
        self.check_remaining_bytes(offset)?;
        // When not in a global peek, peek_cursor == 0 so this is a no-op
        let offset = self.peek_cursor + offset;
        Ok(self.buf[offset])
    }

    pub fn start_peek(&mut self) {
        self.peeking = true;
        self.peek_origins_stack.push(self.peek_cursor);
    }

    /// The peek cursor _from the view of the current innermost peek_.
    pub fn peek_cursor(&self) -> usize {
        self.peek_cursor.checked_sub(*self.peek_origins_stack.last().expect("peek_cursor() was called while no peek is ongoing. This is a bug in the schema display logic.")).expect("The peek cursor offset calculation underflowed. This is a bug in the schema display logic.")
    }

    /// End a peek and proceed with normal input processing.
    /// This method can only be used after a peek is started. If there was no ongoing peek, the
    /// method will panic.
    ///
    /// Returns true if the peek pass is over, false if there is still an outer peek being
    /// processed.
    pub fn end_peek(&mut self) -> bool {
        self.peek_origins_stack.pop().expect("end_peek() was called but the peek stack was empty. This is a bug in the schema display logic.");
        // Only actually end peeking if there are no more nested peeks
        if self.peek_origins_stack.is_empty() {
            self.peeking = false;
            self.peek_cursor = 0;
            true
        } else {
            false
        }
    }
}

/// In case an enum variant contains nested trivial tuples, propagate the virtual-ness transitively
/// to the actual content
fn tuple_displays_as_enum_contents(context: &Context) -> bool {
    context.parent_type == ParentType::Enum(IsHideTag::No)
        || context.parent_type == ParentType::Tuple(IsVirtual::Yes, IsTrivial::Yes)
}

pub struct DisplayVisitor<'a, 'fmt, W> {
    input: Input<'a>,
    output: Output<'fmt, W>,
}

// To avoid unnecessary nested brackets, we apply an optimization to the display of tuples where
// tuples with a single variant do not wrap their contents in parentheses by default.
// This creates a special case when a tuple-variant of an enum has a single field. In that case,
// we need to "make up" the removed delimiter later on in the rendering process.
//
// The "make-up" delimiters are as follows:
// - Inner enum: "."
// - Inner tuple: N/A
// - String-like item: Wrap in parentheses
// - Other: Add an extra " " after the usual delimiter
impl<'a, 'fmt, W> DisplayVisitor<'a, 'fmt, W> {
    pub fn new(input: &'a mut &'a [u8], f: &'fmt mut W) -> Self {
        Self {
            input: Input::new(input),
            output: Output::new(f),
        }
    }

    pub fn has_displayed_whole_input(&self) -> bool {
        self.input.is_empty()
    }

    fn start_peek(&mut self) {
        self.output.start_peek();
        self.input.start_peek();
    }

    fn end_peek(&mut self) -> bool {
        self.output.end_peek();
        self.input.end_peek()
    }

    fn tuple_delimiters<L: LinkingScheme>(
        &self,
        tuple: &Tuple<L>,
        context: &Context,
        schema: &impl TypeResolver<LinkingScheme = L>,
    ) -> Delimiters {
        let first_child_is_primitive = schema
            .resolve_or_err(&tuple.fields[0].value)
            .is_ok_and(|v| v.is_primitive());

        if tuple.fields.len() == 1
            && !(tuple_displays_as_enum_contents(context) && first_child_is_primitive)
        {
            ("", "")
        } else {
            ("(", ")")
        }
    }

    fn enum_delimiters<L: LinkingScheme>(&mut self, _e: &Enum<L>, context: &Context) -> Delimiters {
        match context.parent_type {
            ParentType::Tuple(_, IsTrivial::Yes)
            | ParentType::Enum(_)
            | ParentType::Vec
            | ParentType::Map => (".", ""),
            ParentType::Tuple(_, _) | ParentType::Struct(_) | ParentType::None => ("", ""),
        }
    }

    fn struct_delimiters<L: LinkingScheme>(&self, s: &Struct<L>, context: &Context) -> Delimiters {
        if s.fields.is_empty() {
            return ("", "");
        }
        match context.parent_type {
            ParentType::Tuple(IsVirtual::Yes, _) => (" { ", " }"),
            ParentType::Struct(_)
            | ParentType::None
            | ParentType::Tuple(_, _)
            | ParentType::Vec
            | ParentType::Map => ("{ ", " }"),
            ParentType::Enum(_) => (" { ", " }"),
        }
    }

    /// Generates a template/format string to display a struct if none was provided as an
    /// attribute. The default template either displays the typename if the struct has no
    /// fields, or looks like this (for a hypothetical struct with three fields named as below):
    /// ```text
    /// "{ field_one: {}, field_two: {}, field_three: {} }"
    /// ```
    /// In other words, simply lists out the field names followed by an unnamed substitution (that
    /// will be filled in with the field's values, in order), enclosed in braces and
    /// comma-separated.
    fn struct_default_template<L: LinkingScheme>(
        &self,
        s: &Struct<L>,
        context: &Context,
        schema: &impl TypeResolver<LinkingScheme = L>,
    ) -> String {
        let mut template = String::new();
        let (opener, closer) = self.struct_delimiters(s, context);
        template.push_str(opener);
        if s.fields.is_empty() {
            template.push_str(&s.type_name);
        } else {
            for (i, field) in s.fields.iter().enumerate() {
                if field.silent {
                    continue;
                }
                if schema
                    .resolve_or_err(&field.value)
                    .is_ok_and(|inner| inner.is_skip())
                {
                    continue;
                }
                if i > 0 {
                    template.push_str(self.item_separator());
                }
                template.push_str(&field.display_name);
                template.push_str(": {}");
            }
        }
        template.push_str(closer);
        template
    }

    /// Generates a template/format string to display a tuple if none was provided as an
    /// attribute. The default template looks like this, for example for a tuple with three values:
    /// ```text
    /// "({}, {}, {})"
    /// ```
    /// In other words, simply lists out the fields as unnamed substitutions (which will be filled in
    /// with the tuple's values in order, during display), enclosed in brackets and comma-separated.
    /// The brackets are sometimes omitted - see the implementation of `tuple_delimiters` for the
    /// logic.
    fn tuple_default_template<L: LinkingScheme>(
        &self,
        t: &Tuple<L>,
        context: &Context,
        schema: &impl TypeResolver<LinkingScheme = L>,
    ) -> String {
        let mut template = String::new();
        let (opener, closer) = self.tuple_delimiters(t, context, schema);
        template.push_str(opener);
        for (i, field) in t.fields.iter().enumerate() {
            if field.silent {
                continue;
            }
            if i > 0 {
                template.push_str(self.item_separator());
            }
            template.push_str("{}");
        }
        template.push_str(closer);
        template
    }

    fn option_none_delimiters(&self) -> Delimiters {
        ("None", "")
    }

    fn option_some_delimiters(&self) -> Delimiters {
        ("", "")
    }

    fn map_delimiters(&mut self, context: &Context) -> Delimiters {
        match context.parent_type {
            ParentType::Tuple(IsVirtual::Yes, _) => (" { ", " }"),
            ParentType::Struct(_)
            | ParentType::None
            | ParentType::Tuple(_, _)
            | ParentType::Vec
            | ParentType::Enum(_)
            | ParentType::Map => ("{ ", " }"),
        }
    }

    fn vec_delimiters(&mut self, context: &Context) -> Delimiters {
        match context.parent_type {
            ParentType::Tuple(IsVirtual::Yes, _) => (" [", "]"),
            ParentType::None
            | ParentType::Struct(_)
            | ParentType::Tuple(_, _)
            | ParentType::Vec
            | ParentType::Enum(_)
            | ParentType::Map => ("[", "]"),
        }
    }

    fn item_separator(&self) -> &'static str {
        ", "
    }
}

impl<W: Write> DisplayVisitor<'_, '_, W> {
    pub fn read_usize_borsh(&mut self) -> Result<usize, FormatError> {
        if self.input.len() < 4 {
            return Err(FormatError::MissingIntegerInput {
                claimed_size: 4,
                bytes_available: self.input.len() as u8,
            });
        }
        let len = u32::from_le_bytes(
            self.input
                .advance(4)?
                .try_into()
                .expect("Converting [u8;4] to u32 is infallible"),
        ) as usize;
        Ok(len)
    }

    pub fn display_byte_sequence(
        &mut self,
        len: usize,
        display: ByteDisplay,
        _context: Context,
    ) -> Result<(), FormatError> {
        self.input.check_remaining_bytes(len)?;

        if len > MAX_INPUT_CHUNK {
            display.format(self.input.advance(MAX_INPUT_CHUNK)?, &mut self.output)?;
            self.output.write_fmt(format_args!(
                " (trailing {} bytes truncated)",
                len - MAX_INPUT_CHUNK
            ))?;
            self.input.advance(len - MAX_INPUT_CHUNK)?;
        } else {
            display.format(self.input.advance(len)?, &mut self.output)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Context {
    parent_type: ParentType,
    /// True if this is a first pre-pass that runs to determine field byte lengths before the
    /// actual display pass
    is_peek_pass: bool,
    /// Bytes referenced by other fields.
    /// Maps by field index and offset inside the field, because that is how child fields will
    /// access it.
    peek_bytes: BTreeMap<usize, BTreeMap<usize, u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsVirtual {
    Yes,
    No,
}

/// Tuple wrapping a single value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsTrivial {
    Yes,
    No,
}

/// An enum should display its tags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsHideTag {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentType {
    None,
    Struct(IsVirtual),
    Tuple(IsVirtual, IsTrivial),
    Enum(IsHideTag),
    Vec,
    Map,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            parent_type: ParentType::None,
            is_peek_pass: false,
            peek_bytes: BTreeMap::new(),
        }
    }
}

/// Format an integer number string as a fixed point with the specified number of decimals.
/// E.g.
///  * `format_fixed_point("81", 4)` -> `"0.0081"`
///  * `format_fixed_point("810", 4)` -> `"0.081"`
///  * `format_fixed_point("-81", 4)` -> `"-0.0081"`
///  * `format_fixed_point("24215", 3)` -> `"24.215"`
///  * `format_fixed_point("24200", 3)` -> `"24.2"`
///  * `format_fixed_point("24000", 3)` -> `"24"`
///
/// We pass the number as a string to be able to dynamically support both u128 and
/// i128 in the same function (by stripping the first char of the string if it's a '-').
fn format_fixed_point(number_str: String, decimals: u8) -> String {
    // 1. Strip the negative sign if necessary, to simplify processing
    let (number_str, is_negative) = match number_str.strip_prefix('-') {
        Some(str) => (str.to_string(), true),
        None => (number_str, false),
    };
    // 2. Pad the string to at least decimals + 1, to give a 0 before the '.'
    let mut number_str = format!("{:0>pad$}", number_str, pad = (decimals + 1) as usize);
    // 3. Insert the decimal point `decimals` from the right
    let decimal_idx = number_str.len().checked_sub(decimals.into()).expect("We just formatted the string to be wider than the number of decimals - should never underflow");
    number_str.insert(decimal_idx, '.');
    // 3. Trim trailing 0s, and if necessary, the trailing '.' (if the number ended up being whole)
    let mut number_str = number_str
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string();
    // 4. Finally, restore the negative sign if there was one
    if is_negative {
        number_str.insert(0, '-');
    }
    number_str
}

// TODO: this would be nicer for devex if it were a function. The `<$t>::from_le_bytes` is what
// makes it non-trivial to convert though
macro_rules! display_int {
    ($t:ident, $input:expr, $disp:expr, $f:expr, $ctx:expr) => {{
        let size = IntegerType::$t.size();
        if $input.len() < size {
            return Err(FormatError::MissingIntegerInput {
                claimed_size: size as u8,
                bytes_available: $input.len() as u8,
            });
        }
        let buf = $input.advance(size)?;
        match $disp {
            IntegerDisplay::Hex => {
                write!($f, "{:#x}", <$t>::from_le_bytes(buf.try_into().unwrap()))?
            }
            IntegerDisplay::Decimal => {
                write!($f, "{}", <$t>::from_le_bytes(buf.try_into().unwrap()))?
            }
            IntegerDisplay::FixedPoint(decimal_spec) => {
                let decimals = match decimal_spec {
                    FixedPointDisplay::Decimals(d) => d,
                    FixedPointDisplay::FromSiblingField {
                        field_index,
                        byte_offset,
                    } => {
                        if $ctx.is_peek_pass {
                            0
                        } else {
                            *$ctx
                                .peek_bytes
                                .get(&field_index)
                                .expect("Fixed point display attempted to get field that was not provided in context - this is a bug in the schema display implementation")
                                .get(&byte_offset)
                                .expect("Fixed point display attempted to get byte that was not provided in context - this is a bug in the schema display implementation")
                        }
                    }
                };
                // 39 = log_10(128::MAX)
                if decimals > 39 {
                    return Err(FormatError::TooManyDecimalsForInt (decimals));
                }
                let t = <$t>::from_le_bytes(buf.try_into().unwrap());
                write!($f, "{}", format_fixed_point(t.to_string(), decimals))?
            }
        }
        Ok(())
    }};
}

impl<W: Write, L: LinkingScheme, M> TypeVisitor<L, M> for DisplayVisitor<'_, '_, W> {
    type Arg = Context;
    type ReturnType = Result<(), FormatError>;
    fn visit_enum(
        &mut self,
        e: &Enum<L>,
        schema: &impl TypeResolver<LinkingScheme = L>,
        mut context: Context,
    ) -> Self::ReturnType {
        if self.input.is_empty() && !e.variants.is_empty() {
            return Err(FormatError::MissingDiscriminant {
                type_name: e.type_name.clone(),
            });
        }

        // If the enum is displayed as part of a tuple, the user doesn't have any context about what the type is,
        // since there's no field name. In that case we display the full type name.
        if matches!(context.parent_type, ParentType::Tuple(IsVirtual::No, _))
            || context.parent_type == ParentType::Vec
        {
            self.output.write_str(&e.type_name)?;
        }

        let (open, close) = self.enum_delimiters(e, &context);
        self.output.write_str(open)?;

        let discriminant = self.input.advance(1)?[0];
        let mut variants_by_discriminant =
            e.variants.iter().filter(|v| v.discriminant == discriminant);
        let variant = variants_by_discriminant
            .next()
            .ok_or(FormatError::InvalidDiscriminant {
                type_name: e.type_name.clone(),
                discriminant,
            })?;
        assert!(variants_by_discriminant.next().is_none(), "Found two enum variants with the same discriminant - the schema is malformed, cannot proceed!");
        if variant.template.is_none() && !e.hide_tag {
            write!(self.output, "{}", variant.name)?;
        }
        let is_hide_tag = if e.hide_tag {
            IsHideTag::Yes
        } else {
            IsHideTag::No
        };
        context.parent_type = ParentType::Enum(is_hide_tag);
        if let Some(maybe_resolved) = &variant.value {
            let inner = schema.resolve_or_err(maybe_resolved)?;
            inner.visit(schema, self, context)?;
        }
        self.output.write_str(close)?;
        Ok(())
    }

    fn visit_struct(
        &mut self,
        s: &Struct<L>,
        schema: &impl TypeResolver<LinkingScheme = L>,
        mut context: Context,
    ) -> Self::ReturnType {
        let template = s
            .template
            .clone()
            .unwrap_or_else(|| self.struct_default_template(s, &context, schema));
        let mut template = template.as_str();

        context.parent_type = ParentType::Struct(IsVirtual::No);

        // If we need to get the field offset indices, we perform a peek pass.
        // However, recursive peeks are currently not supported. If a parent struct is currently
        // peeking, skip this: we will perform our pass once it's our turn to display for real.
        // This is to simplify implementation, as being `peekable` is expected to not be used
        // often, and nested peeks should be even more rare. If it becomes a problem, this
        // can be optimised into having a single recursive pre-pass for all structs that need it.
        if s.peekable && !context.is_peek_pass {
            // At which byte does every field start?
            let mut field_offsets: Vec<usize> = Vec::new();
            // For every field, check which bytes need to be provided in the Context.
            // Vec<(field idx, byte offset)>
            let mut bytes_needed: Vec<(usize, usize)> = Vec::new();
            self.start_peek();
            context.is_peek_pass = true;
            for field in s.fields.iter() {
                field_offsets.push(self.input.peek_cursor());
                // TODO: if optimisation is required, this resolution can be cached for the "real"
                // display below to save a clone(). (Need to handle the !peekable case.)
                let inner_ty = schema.resolve_or_err(&field.value)?;
                // TODO: if time should be traded for space, the Output can cache the result of
                // this and reuse it for the "real" display below instead of dropping it
                inner_ty.visit(schema, self, context.clone())?;

                // resolve all bytes necessary from this field
                bytes_needed.extend(inner_ty.parent_byte_references());
            }
            // Save the end of the entire struct, for bounds checking.
            field_offsets.push(self.input.peek_cursor());
            // Rewind the input cursor
            let still_peeking = !self.end_peek();
            context.is_peek_pass = still_peeking;

            // Save, into the Context, those bytes which were found to be needed
            let mut peek_bytes: BTreeMap<usize, BTreeMap<usize, u8>> = BTreeMap::new();
            for (field, offset) in bytes_needed {
                // The upper allowed bound on the offset.
                let field_end = *field_offsets
                    .get(field + 1)
                    .expect("A struct's child field attribute referenced a sibling field by index that is out of bounds of the parent struct. This should not be possible in a well-constructed schema.");
                // unwrap: we just asserted that field_offsets[field+1] exists, so field_offsets[field] must exist too
                let byte_offset = field_offsets.get(field).unwrap() + offset;
                if byte_offset > field_end {
                    return Err(FormatError::FieldReferenceOffsetOutOfBounds(byte_offset));
                }
                let byte = self.input.local_peek_byte(byte_offset)?;

                let field_bytes = peek_bytes.entry(field).or_default();
                field_bytes.insert(offset, byte);
            }
            context.peek_bytes = peek_bytes;
        }

        for field in &s.fields {
            // Save the previous state of the silent flag so we can restore it after displaying the field.
            let was_silent = self.output.silent;
            if field.silent {
                self.output.silent = true;
            }

            let inner_ty = schema.resolve_or_err(&field.value)?;
            if !field.silent && !inner_ty.is_skip() {
                let Some((before_next_field, rest)) = template.split_once("{}") else {
                    return Err(FormatError::InsufficientTemplateSlots);
                };
                self.output.write_str(before_next_field)?;
                template = rest;
            }

            inner_ty.visit(schema, self, context.clone())?;
            // Restore the silent flag to its previous state
            self.output.silent = was_silent;
        }
        if template.contains("{}") {
            return Err(FormatError::UnusedTemplateSlots);
        }
        self.output.write_str(template)?;
        Ok(())
    }

    fn visit_tuple(
        &mut self,
        t: &Tuple<L>,
        schema: &impl TypeResolver<LinkingScheme = L>,
        mut context: Context,
    ) -> Self::ReturnType {
        let template = t
            .template
            .clone()
            .unwrap_or_else(|| self.tuple_default_template(t, &context, schema));
        let mut template = template.as_str();

        let is_virtual = if tuple_displays_as_enum_contents(&context) {
            IsVirtual::Yes
        } else {
            IsVirtual::No
        };
        let trivial = if t.fields.len() == 1 {
            IsTrivial::Yes
        } else {
            IsTrivial::No
        };
        context.parent_type = ParentType::Tuple(is_virtual, trivial);

        // In case we require any field offsets, perform a peeking pass before proper display.
        // Refer to the `visit_struct()` implementation for full documentation.
        if t.peekable && !context.is_peek_pass {
            let mut field_offsets: Vec<usize> = Vec::new();
            // Vec<(field idx, byte offset)>
            let mut bytes_needed: Vec<(usize, usize)> = Vec::new();
            self.start_peek();
            context.is_peek_pass = true;
            for field in t.fields.iter() {
                field_offsets.push(self.input.peek_cursor());
                let inner_ty = schema.resolve_or_err(&field.value)?;
                inner_ty.visit(schema, self, context.clone())?;

                bytes_needed.extend(inner_ty.parent_byte_references());
            }
            field_offsets.push(self.input.peek_cursor());
            let still_peeking = !self.end_peek();
            context.is_peek_pass = still_peeking;

            let mut peek_bytes: BTreeMap<usize, BTreeMap<usize, u8>> = BTreeMap::new();
            for (field, offset) in bytes_needed {
                let field_end = *field_offsets
                    .get(field + 1)
                    .expect("A tuple's child field attribute referenced a sibling field by index that is out of bounds of the parent struct. This should not be possible in a well-constructed schema.");
                // unwrap: we just asserted that field_offsets[field+1] exists, so field_offsets[field] must exist too
                let byte_offset = field_offsets.get(field).unwrap() + offset;
                if byte_offset > field_end {
                    return Err(FormatError::FieldReferenceOffsetOutOfBounds(byte_offset));
                }
                let byte = self.input.local_peek_byte(byte_offset)?;

                let field_bytes = peek_bytes.entry(field).or_default();
                field_bytes.insert(offset, byte);
            }
            context.peek_bytes = peek_bytes;
        }

        for field in &t.fields {
            // Save the previous state of the silent flag so we can restore it after displaying the field.
            let was_silent = self.output.silent;
            if field.silent {
                self.output.silent = true;
            }
            if !field.silent {
                let Some((before_next_field, rest)) = template.split_once("{}") else {
                    return Err(FormatError::InsufficientTemplateSlots);
                };
                self.output.write_str(before_next_field)?;
                template = rest;
            }

            schema
                .resolve_or_err(&field.value)?
                .visit(schema, self, context.clone())?;
            // Restore the silent flag to its previous state
            self.output.silent = was_silent;
        }
        if template.contains("{}") {
            return Err(FormatError::UnusedTemplateSlots);
        }
        self.output.write_str(template)?;
        Ok(())
    }

    fn visit_option(
        &mut self,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        context: Self::Arg,
    ) -> Self::ReturnType {
        let discriminant = self.input.advance(1)?[0];

        match discriminant {
            0 => {
                let (open, close) = self.option_none_delimiters();
                self.output.write_str(open)?;
                self.output.write_str(close)?;
            }
            1 => {
                let (open, close) = self.option_some_delimiters();
                self.output.write_str(open)?;
                schema.resolve_or_err(value)?.visit(schema, self, context)?;
                self.output.write_str(close)?;
            }
            _ => {
                return Err(FormatError::InvalidDiscriminant {
                    type_name: "Option".to_string(),
                    discriminant,
                })
            }
        }

        Ok(())
    }

    fn visit_primitive(
        &mut self,
        p: crate::schema::Primitive,
        _schema: &impl TypeResolver<LinkingScheme = L>,
        context: Context,
    ) -> Self::ReturnType {
        match p {
            Primitive::Float32 => {
                let value = self.input.advance(4)?;
                let value = f32::from_le_bytes(value.try_into().unwrap());
                write!(self.output, "{value}")?;
                Ok(())
            }
            Primitive::Float64 => {
                let value = self.input.advance(8)?;
                let value = f64::from_le_bytes(value.try_into().unwrap());
                write!(self.output, "{value}")?;
                Ok(())
            }
            Primitive::Boolean => {
                let value = self.input.advance(1)?;
                match value[0] {
                    0 => self.output.write_str("false")?,
                    1 => self.output.write_str("true")?,
                    _ => {
                        return Err(FormatError::InvalidDiscriminant {
                            type_name: "bool".to_string(),
                            discriminant: value[0],
                        });
                    }
                }
                Ok(())
            }
            Primitive::Integer(int, display) => match int {
                IntegerType::i8 => display_int!(i8, self.input, display, self.output, context),
                IntegerType::i16 => display_int!(i16, self.input, display, self.output, context),
                IntegerType::i32 => display_int!(i32, self.input, display, self.output, context),
                IntegerType::i64 => display_int!(i64, self.input, display, self.output, context),
                IntegerType::i128 => display_int!(i128, self.input, display, self.output, context),
                IntegerType::u8 => display_int!(u8, self.input, display, self.output, context),
                IntegerType::u16 => display_int!(u16, self.input, display, self.output, context),
                IntegerType::u32 => display_int!(u32, self.input, display, self.output, context),
                IntegerType::u64 => display_int!(u64, self.input, display, self.output, context),
                IntegerType::u128 => display_int!(u128, self.input, display, self.output, context),
            },
            Primitive::ByteArray { len, display } => {
                self.display_byte_sequence(len, display, context)
            }
            Primitive::ByteVec { display } => {
                let len = self
                    .read_usize_borsh()
                    .or(Err(FormatError::MissingVecLength))?;
                self.display_byte_sequence(len, display, context)
            }
            Primitive::String => {
                let len = self
                    .read_usize_borsh()
                    .or(Err(FormatError::MissingStringLength))?;
                let content = self.input.advance(len)?;
                let content = std::str::from_utf8(content)?;
                write!(self.output, "\"{content}\"")?;
                Ok(())
            }
            Primitive::Skip { len } => {
                self.input.advance(len)?;
                Ok(())
            }
        }
    }

    fn visit_array(
        &mut self,
        len: &usize,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        mut context: Context,
    ) -> Self::ReturnType {
        let inner = schema.resolve_or_err(value)?;
        let (open, close) = self.vec_delimiters(&context);
        self.output.write_str(open)?;
        context.parent_type = ParentType::Vec;
        for i in 0..*len {
            if i > 0 {
                self.output.write_str(", ")?;
            }
            inner.visit(schema, self, context.clone())?;
        }
        self.output.write_str(close)?;
        Ok(())
    }

    fn visit_vec(
        &mut self,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        mut context: Context,
    ) -> Self::ReturnType {
        let len = self.read_usize_borsh()?;
        let inner = schema.resolve_or_err(value)?;
        let (open, close) = self.vec_delimiters(&context);
        self.output.write_str(open)?;
        context.parent_type = ParentType::Vec;
        for i in 0..len {
            if i > 0 {
                self.output.write_str(", ")?;
            }
            inner.visit(schema, self, context.clone())?;
        }
        self.output.write_str(close)?;
        Ok(())
    }

    fn visit_map(
        &mut self,
        key: &L::TypeLink,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L>,
        mut context: Context,
    ) -> Self::ReturnType {
        let len = self.read_usize_borsh()?;
        let key = schema.resolve_or_err(key)?;
        let value = schema.resolve_or_err(value)?;
        let (open, close) = self.map_delimiters(&context);
        self.output.write_str(open)?;
        context.parent_type = ParentType::Map;
        for i in 0..len {
            if i > 0 {
                self.output.write_str(", ")?;
            }
            key.visit(schema, self, context.clone())?;
            self.output.write_str(": ")?;
            value.visit(schema, self, context.clone())?;
        }
        self.output.write_str(close)?;
        Ok(())
    }
}
