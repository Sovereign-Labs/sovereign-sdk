use core::panic;
use std::collections::{btree_map, BTreeMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Link;

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TemplateInput {
    type_link: Link,
    offset: usize,
}

impl TemplateInput {
    pub fn type_link(&self) -> &Link {
        &self.type_link
    }

    pub fn offset(&self) -> &usize {
        &self.offset
    }
}

/// Data structure holding a single template for a standard transaction type.
/// Consists of pre-encoded default bytes, and a list of input fields that must be filled in to use
/// the template (with name bindings).
/// During usage, each template input is serialized using the schema accordings to its type, and
/// inserted into the existing template bytes according to its offset index. The final result is a
/// well-formed fully encoded transaction.
/// It is important that input fields must be filled in from last to first (to ensure the `offset`
/// values of earlier fields remain valid - as encoded fields are variable-length and cannot be
/// accounted for).
#[derive(Debug, Default, Clone, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TransactionTemplate {
    preencoded_bytes: Vec<u8>,
    inputs: Vec<(String, TemplateInput)>,
}

impl TransactionTemplate {
    /// Construct a new template from one pre-encoded field.
    /// Used during template generation (normally in the UniversalWallet derive macro), at the
    /// leaf levels of recursive template definition.
    pub fn from_bytes(preencoded_bytes: Vec<u8>) -> Self {
        Self {
            preencoded_bytes,
            inputs: vec![],
        }
    }

    /// Construct a new template from one input field, with a placeholder link type.
    /// While the offset gets fixed in `concat()`, the field_index has to be correct at
    /// construction time.
    /// Used during template generation (normally in the UniversalWallet derive macro), at the
    /// leaf levels of recursive template definition.
    pub fn from_input(name: String, field_index: usize) -> Self {
        Self {
            inputs: vec![(
                name,
                TemplateInput {
                    type_link: Link::IndexedPlaceholder(field_index),
                    offset: 0,
                },
            )],
            preencoded_bytes: vec![],
        }
    }

    pub fn preencoded_bytes(&self) -> &[u8] {
        &self.preencoded_bytes
    }

    pub fn inputs(&self) -> &[(String, TemplateInput)] {
        &self.inputs
    }

    /// Combining template definitions on multiple fields into one template on the parent type.
    /// Concatenation in this context means: pre-encoded bytes are concatenated directly, and inputs
    /// are added to the map in order, with offsets adjusted to accout for earlier pre-encoded
    /// bytes.
    /// Note that placeholder field indexes are NOT adjusted.
    pub fn concat(templates: Vec<Self>) -> Self {
        let mut ret = Self::default();
        let mut prev_bytes_len = 0usize;
        for t in templates {
            ret.preencoded_bytes.extend(t.preencoded_bytes);
            for (name, TemplateInput { type_link, offset }) in t.inputs {
                if ret.inputs.iter().any(|input| input.0 == name) {
                    panic!("Schema transaction template contained duplicate input binding name: {name}");
                }
                ret.inputs.push((
                    name,
                    TemplateInput {
                        type_link,
                        offset: offset + prev_bytes_len,
                    },
                ));
            }
            prev_bytes_len = ret.preencoded_bytes.len();
        }
        ret
    }

    /// Adjust a template definition for a wrapping enum. This means: the discriminant is
    /// prepended to the pre-encoded bytes, and all input offsets are adjusted.
    pub fn prepend_discriminant(&mut self, discriminant: u8) {
        self.preencoded_bytes.insert(0, discriminant);
        for (_, TemplateInput { offset, .. }) in self.inputs.iter_mut() {
            *offset += 1;
        }
    }
}

/// Temporary data structure used to track the origins of each field's template parameters before
/// constructing the parent type's template.
/// This is necessary because subtype-originating templates do not force the parent type to
/// implement this template: the subtype might have been annotated for use elsewhere. A type only
/// inherits a template from subtypes if _every_ subtype implements the template - otherwise, the
/// template is dropped. Only explicitly annotated templates are enforced as mandatory on every
/// field.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AttributeAndChildTemplateSet {
    pub attribute_templates: TransactionTemplateSet,
    pub type_templates: TransactionTemplateSet,
}

/// Data structure denoting a set of templates on a type, indexed by (string) name.
#[derive(Debug, Default, Clone, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TransactionTemplateSet(pub BTreeMap<String, TransactionTemplate>);

impl TransactionTemplateSet {
    /// Replace placeholder links in templates with actual typelinks from the schema (provided by
    /// the type's child_links).
    /// Template placeholders must be `IndexedPlaceholder`s because a template does not necessarily
    /// include placeholders for every field - some fields may be provided with pre-encoded default
    /// values and thus need to be skipped. The index allows looking up the correct type in the
    /// child_links vector.
    pub fn fill_links(
        field_templates: Vec<(String, TransactionTemplate)>,
        child_links: Vec<Link>,
    ) -> Self {
        let mut attribute_templates = BTreeMap::<String, TransactionTemplate>::default();
        for (name, mut template) in field_templates.into_iter() {
            // 1. Fill in any type links
            for (_, input) in template.inputs.iter_mut() {
                match input.type_link {
                    Link::IndexedPlaceholder(i) => input.type_link = child_links[i].clone(),
                    Link::Placeholder => panic!("Templates should not have unindexed Placeholder links. This is a bug in a hand-written implementation."),
                    _ => ()
                }
            }
            // 2. Insert into map, sanity checking for no duplicates
            if let btree_map::Entry::Vacant(e) = attribute_templates.entry(name.clone()) {
                e.insert(template);
            } else {
                panic!("Duplicate template definitions (name: \"{name}\" in attributes. This should never happen (macro should have already errored out).")
            }
        }

        Self(attribute_templates)
    }

    /// Input: a vector of template sets, normally one for each field of a type.
    /// Outputs: a merged template set. This means:
    ///  - Every individual template is concatenated from each field portion of it, across the set
    ///    of templates.
    ///  - The entire set is enforced to be present on every field, so no fields are skipped.
    ///
    /// The exception to the 2nd rule above is for templates only present in the set through field
    /// types (i.e. subtype templates). See the documentation of AttributeAndChildTemplateSet for a
    /// full explanation.
    pub fn concatenate_template_sets(
        template_sets: Vec<AttributeAndChildTemplateSet>,
        type_name_for_diagnostics: &'static str,
    ) -> Self {
        // Eventual return value
        let mut final_template_set = Self::default();

        // The union of elements of each set (as indexed by name) originating directly from
        // attribute annotations on this type's fields
        let attribute_template_names_set: HashSet<_> = template_sets
            .iter()
            .flat_map(|t| t.attribute_templates.0.keys().cloned())
            .collect();
        // The union of elements of each set originating from subtypes...
        let type_template_names_set: HashSet<_> = template_sets
            .iter()
            .flat_map(|t| t.type_templates.0.keys().cloned())
            .collect();
        // And the difference of the two, giving the subtype-only templates
        let type_only_names_set: HashSet<_> = type_template_names_set
            .difference(&attribute_template_names_set)
            .cloned()
            .collect();

        // With the template names tracked, the two components of AttributeAndChildTemplateSet are
        // merged into a single Vec<TransactionTemplateSet> which is then concatenated/merged
        // according to the rules set out above
        let mut zipped_templates: Vec<_> = template_sets.into_iter().map(|mut separated| {
            for (name, template) in separated.type_templates.0.into_iter() {
                if let btree_map::Entry::Vacant(e) = separated.attribute_templates.0.entry(name.clone()) {
                    e.insert(template);
                } else {
                    panic!("Field type's template definitions for \"{}\" overlap with field's own template annotations.", name);
                }
            }
            separated.attribute_templates
        }).collect();

        // Helper function doing the actual merging - as the only difference between the two
        // origins of templates is whether we abort with an error or not on a missing template
        fn collect_concated_templates<F: Fn(&String)>(
            names: HashSet<String>,
            all_templates: &mut [TransactionTemplateSet],
            panic_if_needed: F,
        ) -> TransactionTemplateSet {
            let mut ret = TransactionTemplateSet::default();
            // For every potential element in the set...
            'template: for name in names {
                // build a vector of individual templates...
                let mut template_vec = Vec::<TransactionTemplate>::new();
                for template_set in all_templates.iter_mut() {
                    match template_set.0.remove(&name) {
                        None => {
                            // handle the case where the template is not present on a field
                            panic_if_needed(&name);
                            break 'template;
                        }
                        Some(template) => template_vec.push(template),
                    }
                }
                // finally, with the vector consisting of the sub-components of that template
                // across every single field on our type, concatenate them all into one template -
                // concatenating pre-encoded values, adjusting input offsets etc. - and add it to
                // our return set
                ret.0
                    .insert(name, TransactionTemplate::concat(template_vec));
            }
            ret
        }

        // For templates explicitly annotated on a field of this type, abort immediately if not
        // every field provides this template.
        final_template_set.0.extend(collect_concated_templates(attribute_template_names_set, &mut zipped_templates, |name| {
            panic!("Partial template transaction definition! Metadata for transaction \"{}\" is found on some, but not all, fields of data type {}.\nDouble-check the transaction template attributes and child type definitions. Any template defined on an attribute on any one field must be present for every field, either directly from annotations or from a field's type", name, type_name_for_diagnostics);
        }).0);

        // For templates only inherited from subtypes, ignore any that aren't implemented for every
        // field.
        // Note that calling `final_template_set.extend()` is safe because, earlier, we ensured
        // type_only_names_set has zero intersection with attribute_template_names_set, thus there
        // is no danger of duplicate or overwritten templates: the two `collect_concated_templates`
        // are guaranteed to be disjoint on the space of template names.
        final_template_set.0.extend(
            collect_concated_templates(type_only_names_set, &mut zipped_templates, |_| {}).0,
        );

        final_template_set
    }

    /// Explicitly filter which templates will be part of a given enum variant.
    /// Only templates whose name is in the list will be set on the variant. Additionally, any
    /// names in the list NOT present in the templates will cause an error.
    ///
    /// Optionally, an override can be set to inherit all templates from the variant (primarily
    /// intended for use on the `RuntimeCall`, to inherit all templates from every module's
    /// `CallMessage`). In this case, the filter list is only used for error checking, to
    /// explicitly enforce that all templates named in it MUST be present.
    pub fn filter_enum_variant_templates(
        mut self,
        filter: Vec<String>,
        inherit_all: bool,
        variant_name_for_diagnostics: &'static str,
    ) -> Self {
        let mut filter_set: HashSet<String> = HashSet::from_iter(filter);
        self.0
            .retain(|name, _| filter_set.remove(name) || inherit_all);
        if !filter_set.is_empty() {
            // Unwrap: we know the set isn't empty, so it must have at least one entry
            panic!("Enum variant {variant_name_for_diagnostics} specified template \"{}\" which was not defined on the variant's fields", filter_set.iter().next().unwrap());
        }
        self
    }

    pub fn merge_enum_template_sets(
        template_sets: Vec<TransactionTemplateSet>,
        type_name_for_diagnostics: &'static str,
    ) -> Self {
        // Eventual return value
        let mut final_template_set = Self::default();

        // For every variant (one entry in the input vec), ensure none of the templates were
        // already seen in previous variants, then adjust the template for the discriminant and add
        // it to the output
        for (discriminant, set) in template_sets.into_iter().enumerate() {
            for (name, mut template) in set.0.into_iter() {
                if final_template_set.0.contains_key(&name) {
                    panic!("Different variants of the enum {} both contained definitions for the transaction template \"{}\". At this time, only one enum branch can be defined for any transaction template.", type_name_for_diagnostics, name);
                }
                template.prepend_discriminant(
                    discriminant
                        .try_into()
                        .expect("Enum discriminants are only supported up to size u8"),
                );
                final_template_set.0.insert(name, template);
            }
        }

        final_template_set
    }
}
