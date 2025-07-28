use thiserror::Error;

use super::{ContainerSerdeMetadata, Enum, LinkingScheme, Struct, Tuple, Ty};
use crate::schema::{IndexLinking, Primitive, Schema};

pub trait TypeVisitor<L: LinkingScheme, M> {
    type Arg;
    type ReturnType;
    fn visit_enum(
        &mut self,
        e: &Enum<L>,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
    fn visit_struct(
        &mut self,
        s: &Struct<L>,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
    fn visit_tuple(
        &mut self,
        t: &Tuple<L>,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
    fn visit_option(
        &mut self,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
    fn visit_primitive(
        &mut self,
        p: Primitive,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
    fn visit_vec(
        &mut self,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
    fn visit_array(
        &mut self,
        len: &usize,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
    fn visit_map(
        &mut self,
        key: &L::TypeLink,
        value: &L::TypeLink,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        context: Self::Arg,
    ) -> Self::ReturnType;
}

pub trait TypeResolver {
    type LinkingScheme: LinkingScheme;
    type Metadata;

    fn resolve_or_err(
        &self,
        maybe_resolved: &<Self::LinkingScheme as LinkingScheme>::TypeLink,
    ) -> Result<Ty<Self::LinkingScheme>, ResolutionError>;

    fn maybe_resolve_metadata(
        &self,
        _maybe_resolved: &<Self::LinkingScheme as LinkingScheme>::TypeLink,
    ) -> Result<Option<Self::Metadata>, ResolutionError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ResolutionError {
    #[error("Could not resolve type with index `{index}` (max: `{max}`)")]
    IndexOutOfBounds { index: usize, max: usize },
    #[error("The schema contained an unresolved placholder")]
    ErrContainsPlaceholder,
}

impl TypeResolver for Schema {
    type LinkingScheme = IndexLinking;
    type Metadata = ContainerSerdeMetadata;

    fn resolve_or_err(
        &self,
        maybe_resolved: &<Self::LinkingScheme as LinkingScheme>::TypeLink,
    ) -> Result<Ty<Self::LinkingScheme>, ResolutionError> {
        match maybe_resolved {
            crate::schema::Link::ByIndex(idx) => {
                self.types()
                    .get(*idx)
                    .cloned()
                    .ok_or(ResolutionError::IndexOutOfBounds {
                        index: *idx,
                        max: self.types().len(),
                    })
            }
            crate::schema::Link::Immediate(primitive) => Ok(primitive.clone().into()),
            crate::schema::Link::Placeholder | crate::schema::Link::IndexedPlaceholder(_) => {
                Err(ResolutionError::ErrContainsPlaceholder)
            }
        }
    }

    fn maybe_resolve_metadata(
        &self,
        maybe_resolved: &<Self::LinkingScheme as LinkingScheme>::TypeLink,
    ) -> Result<Option<Self::Metadata>, ResolutionError> {
        match maybe_resolved {
            crate::schema::Link::ByIndex(idx) => {
                Some(self.serde_metadata().get(*idx).cloned().ok_or(
                    ResolutionError::IndexOutOfBounds {
                        index: *idx,
                        max: self.types().len(),
                    },
                ))
                .transpose()
            }
            crate::schema::Link::Immediate(_) => Ok(None),
            crate::schema::Link::Placeholder | crate::schema::Link::IndexedPlaceholder(_) => {
                Err(ResolutionError::ErrContainsPlaceholder)
            }
        }
    }
}

impl<L: LinkingScheme> Ty<L> {
    pub fn visit<V: TypeVisitor<L, M>, M>(
        &self,
        schema: &impl TypeResolver<LinkingScheme = L, Metadata = M>,
        visitor: &mut V,
        arg: V::Arg,
    ) -> V::ReturnType {
        match self {
            Ty::Boolean => visitor.visit_primitive(Primitive::Boolean, schema, arg),
            Ty::Enum(e) => visitor.visit_enum(e, schema, arg),
            Ty::Struct(s) => visitor.visit_struct(s, schema, arg),
            Ty::Tuple(t) => visitor.visit_tuple(t, schema, arg),
            Ty::Option { value } => visitor.visit_option(value, schema, arg),
            Ty::Integer(kind, display) => {
                visitor.visit_primitive(Primitive::Integer(*kind, *display), schema, arg)
            }
            Ty::ByteArray { len, display } => visitor.visit_primitive(
                Primitive::ByteArray {
                    len: *len,
                    display: *display,
                },
                schema,
                arg,
            ),
            Ty::ByteVec { display } => {
                visitor.visit_primitive(Primitive::ByteVec { display: *display }, schema, arg)
            }
            Ty::Array { len, value } => visitor.visit_array(len, value, schema, arg),
            Ty::Float32 => visitor.visit_primitive(Primitive::Float32, schema, arg),
            Ty::Float64 => visitor.visit_primitive(Primitive::Float64, schema, arg),
            Ty::Map { key, value } => visitor.visit_map(key, value, schema, arg),
            Ty::Vec { value } => visitor.visit_vec(value, schema, arg),
            Ty::String => visitor.visit_primitive(Primitive::String, schema, arg),
            Ty::Skip { len } => visitor.visit_primitive(Primitive::Skip { len: *len }, schema, arg),
        }
    }
}
