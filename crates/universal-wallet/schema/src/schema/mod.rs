use std::any::TypeId;
use std::collections::HashMap;
use std::fmt::Debug;

use once_cell::sync::OnceCell;

use sha2::{Digest, Sha256};
pub mod container;
mod primitive;
pub mod safe_string;
pub mod transaction_templates;
use borsh::{BorshDeserialize, BorshSerialize};
pub use container::Container;
use nmt_rs::simple_merkle::db::MemDb;
use nmt_rs::simple_merkle::tree::MerkleTree;
use nmt_rs::TmSha2Hasher;
pub use primitive::Primitive;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
mod schema_impls;

use thiserror::Error;
use transaction_templates::TransactionTemplateSet;

use crate::display::{Context as DisplayContext, DisplayVisitor, FormatError};
#[cfg(feature = "serde")]
use crate::json_to_borsh::{Context as EncodeContext, EncodeError, EncodeVisitor};
use crate::ty::byte_display::ByteParseError;
use crate::ty::{ContainerSerdeMetadata, LinkingScheme, Ty};
#[cfg(feature = "eip712")]
use crate::visitors::eip712::{Context as Eip712Context, Eip712Error, Eip712Visitor};
#[cfg(feature = "eip712")]
use alloy_dyn_abi::{Eip712Types, Error as AlloyEip712Error, TypedData};

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error(transparent)]
    FormatError(#[from] FormatError),
    #[error(transparent)]
    BorshError(#[from] borsh::io::Error),
    #[cfg(feature = "serde")]
    #[error(transparent)]
    EncodeError(#[from] EncodeError),
    #[cfg(feature = "serde")]
    #[error(transparent)]
    JsonError(#[from] serde_json::Error),
    #[cfg(feature = "eip712")]
    #[error(transparent)]
    Eip712Error(#[from] Eip712Error),
    #[cfg(feature = "eip712")]
    #[error(transparent)]
    AlloyEip712Error(#[from] AlloyEip712Error),
    #[error(transparent)]
    Bech32Error(#[from] ByteParseError),
    #[error("Rollup type {0:?} was missing from schema")]
    MissingRollupRoot(RollupRoots),
    #[error("Template {0} not found in schema")]
    UnknownTemplate(String),
    #[error("Index {0} not found in schema")]
    InvalidIndex(usize),
    #[error("Metadata hash must be provided but was not initialized. The schema was not properly finalized, or the serialized schema was invalid.")]
    MetadataHashNotInitialized,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IndexLinking;

impl LinkingScheme for IndexLinking {
    type TypeLink = Link;
}

// TODO: Some type safety for fully-constructed schemas.
// It should be possible to use the type system to ensure at compile-time that
// a) constructed Schemas do not have any Link::Placeholder; and
// b) it is not possible to call construction methods (the ones that edit the links) on a finished
// Schema.
// This could be done with, for example, an intermediate SchemaUnderConstruction type using a
// ConstrutionIndexLinking, which implements into::<Schema>().
//
// Right now this is mostly achieved using member visibility (nobody outside can call the private
// construction methods) and sanity checking (on a derived schema, if under_construction is empty,
// there won't be any placeholders); but a separate type would provide a stronger guarantee.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Link {
    ByIndex(usize),
    Immediate(Primitive),
    Placeholder,
    /// Placeholder indexed by its place in the parent datastructure
    IndexedPlaceholder(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaybePartialLink {
    Partial(Link),
    Complete(Link),
}

impl MaybePartialLink {
    fn into_inner(self) -> Link {
        match self {
            MaybePartialLink::Partial(link) => link,
            MaybePartialLink::Complete(link) => link,
        }
    }
}

/// This newtype is mainly necessary to allow the schema to derive Debug ergonomically
/// Stores both the tree and its root since MerkleTree::root() requires &mut self
pub struct ConstructedMerkleTree(OnceCell<(MerkleTree<MemDb<[u8; 32]>, TmSha2Hasher>, [u8; 32])>);

impl Default for ConstructedMerkleTree {
    fn default() -> Self {
        Self(OnceCell::new())
    }
}

impl Debug for ConstructedMerkleTree {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

/// This extra metadata is used in contexts where serde features are enabled; thus, we do not
/// serialize it in serde formats, as it should be recomputed before using the data committed
/// using it. (Otherwise a frontend could supply malicious metadata and a mismatching hash to
/// make the hash pass chain ID checks.)
/// When serde is disabled (i.e. usecases using borsh serialization), this metadata is unused, so
/// the committment is the only relevant information and a mismatch is not possible.
/// TL;DR:
/// - In borsh: serializes/deserializes the actual hash value, while corresponding metada is empty
/// - In serde: skips serialization and recalculates on first use, ensuring hash matches the
///   deserialized metadata
struct MetadataHash(OnceCell<[u8; 32]>);

impl Default for MetadataHash {
    fn default() -> Self {
        Self(OnceCell::new())
    }
}

impl Debug for MetadataHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

impl BorshSerialize for MetadataHash {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // The hash must be calculated before serialization (via finalize())
        // It's an error to serialize a schema that hasn't been finalized
        let hash = self.0.get().copied()
            .ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Cannot serialize Schema: metadata_hash not initialized. Call finalize() before serializing"
            ))?;
        BorshSerialize::serialize(&hash, writer)
    }
}

impl BorshDeserialize for MetadataHash {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let hash: [u8; 32] = BorshDeserialize::deserialize_reader(reader)?;
        let metadata_hash = MetadataHash::default();
        // Set the hash - this should always succeed for a new MetadataHash
        metadata_hash.0.set(hash).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to set metadata_hash in OnceCell during deserialization",
            )
        })?;
        Ok(metadata_hash)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemId(pub TypeId);

impl ItemId {
    pub fn of<T: 'static + UniversalWallet>() -> Self {
        T::id_override().unwrap_or(ItemId(TypeId::of::<T>()))
    }
}

/// Not enforced in the types, but the expected convention that should be followed when generating
/// the schema.
#[derive(Debug, Copy, Clone)]
pub enum RollupRoots {
    Transaction = 0,
    UnsignedTransaction = 1,
    RuntimeCall = 2,
    Address = 3,
}

/// The standard metadata format for every chain. Includes a numeric chain_id and a human-readable
/// chain name.
#[derive(Debug, Default, Clone, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ChainData {
    pub chain_id: u64,
    pub chain_name: String,
}

/// A schema, representing set of types (i.e. rust code) as a data structure.
/// The schema allows any included type's borsh serialization to be displayed as a human readable string,
/// and the type's JSON serialisation to be re-serialised to borsh without depending on the
/// original Rust type.
/// It is also serialisable and therefore, once generated for a rollup, can be imported and used with
/// non-Rust languages, enabling toolkits in any language to implement the same functionality as above.
///
/// A schema can be instantiated for any type that implements either `UniversalWallet` or
/// `OverrideSchema`. In turn, `UniversalWallet` is intended to be automatically derived using the
/// `UniversalWallet` macro.
#[derive(Default, Debug, BorshSerialize, BorshDeserialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Schema {
    /// The types described by this schema. This is an array of type descriptions, where complex
    /// types refer to their sub-types by index within the array.
    /// Any of the types here can be used for schema operations (such as borsh-to-human or
    /// json-to-borsh reserialisations).
    types: Vec<Ty<IndexLinking>>,

    /// A mapping from the complex root types that parametrized the schema generation invocation (in
    /// order, skipping primitives) to the actual indices they ended up at in the type array above.
    root_type_indices: Vec<usize>,

    /// Global metadata for the chain.
    chain_data: ChainData,

    /// Extra metadata hash. "Extra" metadata is defined as metadata irrelevant when no additional
    /// features are enabled.
    #[cfg_attr(feature = "serde", serde(skip))]
    extra_metadata_hash: MetadataHash,

    /// The chain hash: the top-level hash committing to the entire schema, including all types and
    /// all metadata.
    /// This should be recalculated independently whenever the schema is used, thus is not included
    /// in serializations. This field caches the results of the calculation for subsequent uses
    /// after the schema has been deserialized and constructed.
    #[cfg_attr(feature = "serde", serde(skip))]
    #[borsh(skip)]
    chain_hash: OnceCell<[u8; 32]>,

    /// A list of templatable objects that can be constructed from standard input, per root type (in
    /// order corresponding to root_type_indices). Mapped by template name.
    /// Should be skipped in binary serialisation for hardware wallet apps.
    #[borsh(skip)]
    templates: Vec<TransactionTemplateSet>,

    /// A set of metadata items for each field in the `types` vec, used for serde-compatible
    /// deserialisation (i.e. `json_to_borsh()`).
    /// It is separated from the main vec of `Ty` structs to allow non-serde implementations of the
    /// schema, meaning ones only concerned with `borsh`-serialized interpretation (i.e.
    /// `display()` functionality), such as hardware wallets, to avoid deserializing this. Not only
    /// does this save resources but it also allows the format to be modified to implement
    /// additional serde compatibility features without causing breaking changes for non-serde
    /// implementations.
    #[borsh(skip)]
    serde_metadata: Vec<ContainerSerdeMetadata>,

    /// Cached (lazily-constructed) merkelization of the entire schema.
    #[cfg_attr(feature = "serde", serde(skip))]
    #[borsh(skip)]
    merkle_tree: ConstructedMerkleTree,

    /// A map from the type ID of an item to its index in the types array. Note that primitives and "virtual" structs/tuples
    /// (i.e. the contents of an enum variant) are not included in this map.
    /// Only used during schema construction.
    #[cfg_attr(feature = "serde", serde(skip))]
    #[borsh(skip)]
    known_types: HashMap<ItemId, usize>,

    /// Keeps track of all the types which are partially constructed. By the end of schema generation, this
    /// must be empty.
    #[cfg_attr(feature = "serde", serde(skip))]
    #[borsh(skip)]
    under_construction: HashMap<ItemId, usize>,
}

impl Schema {
    /// Instantiate a schema for a single type.
    /// This root type will be at index 0
    pub fn of_single_type<T: UniversalWallet>() -> Result<Self, SchemaError> {
        // TODO: this could easily be implemented with a macro for N types for any N >= 1, if ever needed
        let mut schema = Self::default();
        T::make_root_of(&mut schema);
        schema.finalize()?;
        Ok(schema)
    }

    /// Instantiate a schema for a standard set of rollup types: its complete transaction, its
    /// unsigned transaction, and its call message type.
    /// The types will be accessible using the indices stored in root_type_indices (in the above
    /// order); they can also be queried using the `RollupRoots` enum through the `_rollup`-tagged
    /// functions on the schema
    pub fn of_rollup_types_with_chain_data<
        Transaction: UniversalWallet,
        UnsignedTransaction: UniversalWallet,
        RuntimeCall: UniversalWallet,
        Address: UniversalWallet,
    >(
        chain_data: ChainData,
    ) -> Result<Self, SchemaError> {
        let mut schema = Schema {
            chain_data,
            ..Self::default()
        };
        Transaction::make_root_of(&mut schema);
        UnsignedTransaction::make_root_of(&mut schema);
        RuntimeCall::make_root_of(&mut schema);
        Address::make_root_of(&mut schema);

        schema.finalize()?;
        Ok(schema)
    }

    #[cfg(not(feature = "serde"))]
    pub fn metadata_hash(&self) -> Result<[u8; 32], SchemaError> {
        // In borsh-only context, the hash must have been deserialized
        // If it's not present, that's a critical error
        self.extra_metadata_hash
            .0
            .get()
            .copied()
            .ok_or(SchemaError::MetadataHashNotInitialized)
    }

    #[cfg(feature = "serde")]
    pub fn metadata_hash(&self) -> Result<[u8; 32], SchemaError> {
        // In serde context, calculate on first use if not present
        self.extra_metadata_hash
            .0
            .get_or_try_init(|| self.calculate_metadata_hash())
            .copied()
    }

    #[cfg(feature = "serde")]
    fn calculate_metadata_hash(&self) -> Result<[u8; 32], SchemaError> {
        let mut hasher = Sha256::new();
        hasher.update(&borsh::to_vec(&self.templates)?);
        hasher.update(&borsh::to_vec(&self.serde_metadata)?);
        Ok(hasher.finalize().into())
    }

    #[cfg(feature = "serde")]
    pub fn from_json(input: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(input)
    }

    pub fn rollup_expected_index(&self, rollup_type: RollupRoots) -> Result<usize, SchemaError> {
        self.root_type_indices
            .get(rollup_type as usize)
            .copied()
            .ok_or(SchemaError::MissingRollupRoot(rollup_type))
    }

    /// Use the schema to display the given type using the provided borsh encoded input
    pub fn display(&self, type_index: usize, input: &[u8]) -> Result<String, SchemaError> {
        let mut output = String::new();
        let input = &mut &input[..];
        let mut visitor = DisplayVisitor::new(input, &mut output);
        self.types
            .get(type_index)
            .ok_or(SchemaError::InvalidIndex(type_index))?
            .visit(self, &mut visitor, DisplayContext::default())?;

        if !visitor.has_displayed_whole_input() {
            return Err(FormatError::UnusedInput.into());
        }
        Ok(output)
    }

    #[cfg(feature = "eip712")]
    pub fn eip712_json(&self, type_index: usize, input: &[u8]) -> Result<String, SchemaError> {
        let Some(typed_data) = self.eip712_get_typed_data_inner(type_index, input)? else {
            return Ok(String::default());
        };
        Ok(serde_json::to_string(&typed_data)?)
    }

    #[cfg(feature = "eip712")]
    pub fn eip712_signing_hash(
        &self,
        type_index: usize,
        input: &[u8],
    ) -> Result<[u8; 32], SchemaError> {
        let Some(typed_data) = self.eip712_get_typed_data_inner(type_index, input)? else {
            return Ok(Default::default());
        };
        Ok(typed_data.eip712_signing_hash()?.into())
    }

    #[cfg(feature = "eip712")]
    fn eip712_get_typed_data_inner(
        &self,
        type_index: usize,
        input: &[u8],
    ) -> Result<Option<TypedData>, SchemaError> {
        let mut out_types = Eip712Types::default();
        let input = &mut &input[..];
        let mut visitor = Eip712Visitor::new(input, &mut out_types);
        let root_type = self
            .types()
            .get(type_index)
            .ok_or(SchemaError::InvalidIndex(type_index))?;
        let Some(visitor_return) = root_type.visit(self, &mut visitor, Eip712Context::default())?
        else {
            return Ok(None);
        };
        if !visitor.has_displayed_whole_input() {
            return Err(FormatError::UnusedInput.into());
        }

        Ok(Some(TypedData {
            domain: alloy_dyn_abi::Eip712Domain {
                name: Some(self.chain_data.chain_name.clone().into()),
                version: None,
                chain_id: Some(alloy_primitives::U256::from(self.chain_data.chain_id)),
                // Our chain hash is 32 bytes. We could truncate it to fit in the 20-byte ethereum
                // Address, but by putting it in the salt we retain the full entropy and security.
                verifying_contract: None,
                salt: Some(self.chain_hash()?.into()),
            },
            resolver: out_types.into(),
            primary_type: visitor_return.unique_type_name,
            message: visitor_return.json_value,
        }))
    }

    /// Use the schema to convert a serde-compatible JSON string of the given type into its borsh
    /// encoding
    #[cfg(feature = "serde")]
    pub fn json_to_borsh(&self, type_index: usize, input: &str) -> Result<Vec<u8>, SchemaError> {
        let mut output = Vec::new();

        let mut visitor = EncodeVisitor::new(&mut output)?;

        self.types
            .get(type_index)
            .ok_or(SchemaError::InvalidIndex(type_index))?
            .visit(self, &mut visitor, EncodeContext::new(input, type_index)?)?;

        Ok(output)
    }

    /// Use a stub JSON to create a full type using the named template.
    #[cfg(feature = "serde")]
    pub fn fill_template_from_json(
        &self,
        root_index: usize,
        template_name: &str,
        input: &str,
    ) -> Result<Vec<u8>, SchemaError> {
        fn serde_to_schema_err(e: serde_json::Error) -> SchemaError {
            SchemaError::EncodeError(EncodeError::Json(e.to_string()))
        }

        let template = self
            .templates
            .get(root_index)
            .ok_or(SchemaError::InvalidIndex(root_index))?
            .0
            .get(template_name)
            .ok_or(SchemaError::UnknownTemplate(template_name.to_string()))?;

        // Parse the JSON as a map/object of inputs
        let mut input_map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(input)
                .map_err(serde_to_schema_err)?;

        let mut output = template.preencoded_bytes().to_owned();

        // For every input in the template, starting from the end (to preserve the `offset` values
        // of previous inputs)...
        for (name, input) in template.inputs().iter().rev() {
            // ...get its type from the schema,
            let ty = match input.type_link() {
                Link::ByIndex(i) => self.types.get(*i).expect("Template {name} contained an invalid link: {i}. This is a major bug with template generation."),
                Link::Immediate(ty) => &ty.clone().into(),
                Link::Placeholder | Link::IndexedPlaceholder(_) => panic!("Template {name} contained placeholder link. This is a major bug with template generation.")
            };
            // find the corresponding JSON value,
            let json_value = input_map.remove(name).ok_or(EncodeError::MissingType {
                name: name.to_owned(),
            })?;
            // and use our json_to_borsh functionality to get the bytes for the input.
            let mut buf = Vec::new();
            let mut visitor = EncodeVisitor::new(&mut buf)?;
            ty.visit(
                self,
                &mut visitor,
                EncodeContext::from_val(json_value, input.type_link()),
            )?;

            // Finally, splice the obtained bytes at the specified offset into the template.
            output.splice(input.offset()..input.offset(), buf);
        }

        if !input_map.is_empty() {
            // Unwrap: we know input_map isn't empty, so it must have at least one entry
            return Err(SchemaError::EncodeError(EncodeError::UnusedInput {
                value: input_map.iter().next().unwrap().0.to_owned(),
            }));
        }

        Ok(output)
    }

    /// Lists all templates available for the given root type.
    #[cfg(feature = "serde")]
    pub fn templates(&self, index: usize) -> Result<Vec<String>, SchemaError> {
        Ok(self
            .templates
            .get(index)
            .ok_or(SchemaError::InvalidIndex(index))?
            .0
            .keys()
            .cloned()
            .collect())
    }

    /// Returns the chain ID calculated using the merkle root of all the schema types, combined
    /// with any chain-specific metadata.
    /// This allows the chain ID to be used for verification of the schema (and thus verification
    /// that a transaction claiming to correspond to a given schema will have the effect it claims).
    pub fn chain_hash(&self) -> Result<[u8; 32], SchemaError> {
        self.chain_hash
            .get_or_try_init(|| {
                // First, merkleize the schema
                let merkle_root = self.merkle_root()?;

                // Then, hash the auxilliary internal data - root indices and chain data
                let mut hasher = Sha256::new();
                hasher.update(&borsh::to_vec(&self.root_type_indices)?);
                hasher.update(&borsh::to_vec(&self.chain_data)?);
                let internal_data_hash: [u8; 32] = hasher.finalize().into();

                // Get the metadata hash
                let metadata_hash = self.metadata_hash()?;

                // Finally, combine the three hashes in order to get the final chain hash
                let mut hasher = Sha256::new();
                hasher.update(merkle_root);
                hasher.update(internal_data_hash);
                hasher.update(metadata_hash);

                let chain_hash: [u8; 32] = hasher.finalize().into();
                Ok(chain_hash)
            })
            .copied()
    }

    fn merkle_root(&self) -> Result<[u8; 32], SchemaError> {
        let (_, root) = self.merkle_tree.0.get_or_try_init(|| {
            let mut tree = MerkleTree::new();
            for ty in &self.types {
                tree.push_raw_leaf(&borsh::to_vec(ty)?)
            }
            let root = tree.root();
            Ok::<_, SchemaError>((tree, root))
        })?;
        Ok(*root)
    }

    fn finalize(&self) -> Result<(), SchemaError> {
        // Ensure both hashes are calculated and cached
        self.metadata_hash()?;
        self.chain_hash()?;
        Ok(())
    }

    pub fn types(&self) -> &[Ty<IndexLinking>] {
        &self.types
    }

    pub fn serde_metadata(&self) -> &[ContainerSerdeMetadata] {
        &self.serde_metadata
    }

    pub fn root_types(&self) -> &[usize] {
        &self.root_type_indices
    }

    fn find_item_by_id(&self, item_id: &ItemId) -> Option<usize> {
        self.known_types.get(item_id).copied()
    }

    /// Link a child type to its parent, panicking if the parent type is not in the schema or if the parent type has no more placeholders.
    fn link_child_to_parent(&mut self, parent: ItemId, child: Link) {
        let idx = self.known_types.get(&parent).unwrap_or_else(|| panic!("Tried to link a child to a parent ({parent:?}) that the schema doesn't have. This is a bug in a hand-written schema."));

        let remaining_children = *self.under_construction.get(&parent).unwrap_or_else(|| panic!("Tried to link too many children to parent ({parent:?}). This is a bug in a hand-written schema."));
        if remaining_children == 1 {
            self.under_construction.remove(&parent);
        } else {
            self.under_construction
                .insert(parent, remaining_children - 1);
        }
        self.types[*idx].fill_next_placholder(child);
    }

    /// Get a link to the given type, adding it to the top-level schema if necessary.
    /// Unlike all other methods in this crate, the linked type returned by this method is allowed to be only partially generated.
    ///
    /// It is the responsibility of the caller to complete the returned link.
    fn get_partial_link_to(
        &mut self,
        item: Item<IndexLinking>,
        item_id: ItemId,
    ) -> MaybePartialLink {
        match item {
            Item::Container(c) => {
                if let Some(location) = self.find_item_by_id(&item_id) {
                    MaybePartialLink::Complete(Link::ByIndex(location))
                } else {
                    let num_children = c.num_children();
                    let serde_metadata = c.serde();
                    let location = self.types.len();
                    self.known_types.insert(item_id.clone(), location);
                    self.types.push(c.into());
                    self.serde_metadata.push(serde_metadata);
                    if num_children != 0 {
                        self.under_construction.insert(item_id, num_children);
                        MaybePartialLink::Partial(Link::ByIndex(location))
                    } else {
                        MaybePartialLink::Complete(Link::ByIndex(location))
                    }
                }
            }
            Item::Atom(primitive) => MaybePartialLink::Complete(Link::Immediate(primitive)),
        }
    }

    /// After generating a root type, register it with the schema for ease of reference. Sets
    /// the canonical "root links", so has to be carefully called in the right order (normally,
    /// immediately after root type construction, with the link to the newly created type).
    /// No-op for primitive links.
    /// Panics on placeholder links.
    fn push_root_link(&mut self, link: Link) {
        match link {
            Link::ByIndex(i) => self.root_type_indices.push(i),
            Link::Immediate(..) => {},
            Link::Placeholder | Link::IndexedPlaceholder(_) => panic!("Attempted to register a placeholder link as a schema root - are you passing the right link?"),
        }
    }
}

pub enum Item<L: LinkingScheme> {
    Container(Container<L>),
    Atom(Primitive),
}

/// Generate the schema for a type.
/// For complex types, this should typically be derived with a macro,
/// rather than implemented by hand.
/// This is also automatically implemented for all types implementing `OverrideSchema`.
pub trait UniversalWallet: Sized + 'static {
    /// Ensure that each type contained in the outer type (i.e. the type of each struct/tuple field) is added to the schema,
    /// and return a `Link` connecting the child to the parent.
    ///
    /// Ideally, this function would return something like `Box<dyn UniversalWallet>`.
    /// Unfortunately, we need to return *types*, not instances (because we don't want to
    /// add a `Default` bound on all types that implement UniversalWallet) which Rustc doesn't like.
    /// So, we have a slightly messier signature where the type is expected to register each of its child
    /// types with the schema directly rather than returning them to the caller for future registration.
    fn get_child_links(schema: &mut Schema) -> Vec<Link>;

    /// Generate the "scaffolding" of the item. If the item is a primtive, this is just the corresponding primtive.
    /// If the type is composed of other types, this is the container with all links set to [`Link::Placeholder`].
    fn scaffold() -> Item<IndexLinking>;

    /// Writes the type to the schema if it is not already present and returns a link to it.
    ///
    /// Any child types will have their schemas generated as well, but the placement of those types is left
    /// to the discretion of the implementation - they may or may not appear at the top level of the schema.
    fn write_schema(schema: &mut Schema) -> Link {
        let item = Self::scaffold();
        let item_id = ItemId::of::<Self>();
        match item {
            Item::Atom(_primitive) => {
                // When recursively building the schema, primitives get filled in directly as
                // Link::Immediate and do not get `write_schema` called for them. Thus this can
                // only happen from a user call.
                // Forbidding this makes managing metadata significantly easier.
                panic!("Creating a schema for primitive root types is not supported. If this is necessary, wrap the primitive in a newtype struct. If you did not specify a primitive root type, this may be a bug in schema generation.");
            }
            Item::Container(container) => {
                let link = schema.get_partial_link_to(Item::Container(container), item_id.clone());
                if let MaybePartialLink::Complete(link) = link {
                    return link;
                }

                for child in Self::get_child_links(schema) {
                    schema.link_child_to_parent(item_id.clone(), child);
                }
                link.into_inner()
            }
        }
    }

    /// Writes the type and all its children to the schema, if not already present, and sets the
    /// type as a root type. Generates any templates defined on that type.
    fn make_root_of(schema: &mut Schema) {
        let link = Self::write_schema(schema);
        assert!(
            schema.under_construction.is_empty(),
            "Schema generation left some types partially constructed. This is a bug in the schema. {schema:?}"
        );
        schema.push_root_link(link);
        let templates = Self::get_child_templates(schema);
        schema.templates.push(templates);
    }

    /// Empty by default
    /// When derived by the macro, builds a template set from annotations on the fields + the field
    /// types' own get_child_templates()
    fn get_child_templates(_schema: &mut Schema) -> TransactionTemplateSet {
        Default::default()
    }

    /// Gets a link to the type, writing the type to the schema if necessary.
    fn make_linkable(schema: &mut Schema) -> Link {
        match Self::scaffold() {
            Item::Container(_) => Self::write_schema(schema),
            Item::Atom(atom) => Link::Immediate(atom),
        }
    }

    /// Override the type ID of the item. This should typically not be written by hand. Instead,
    /// use the [`OverrideSchema`] trait.
    fn id_override() -> Option<ItemId> {
        None
    }
}

/// Establish that this type should use the `Output` type to generate its schema.
/// This is appropriate for cases where different types represent the same kind of data structure.
/// For instance, HashMap and BTreeMap both represent a `Container::Map` in the data model of the
/// schema; their internal implementation differences don't affect the shape of their schemas.
///
/// Note that, for types to be considered equivalent in the schema, their borsh and JSON
/// serialisations must both also be equivalent.
pub trait OverrideSchema {
    type Output: UniversalWallet;
}

impl<T: OverrideSchema + 'static> UniversalWallet for T {
    fn scaffold() -> Item<IndexLinking> {
        <Self as OverrideSchema>::Output::scaffold()
    }
    fn get_child_links(schema: &mut Schema) -> Vec<Link> {
        <Self as OverrideSchema>::Output::get_child_links(schema)
    }
    fn id_override() -> Option<ItemId> {
        <Self as OverrideSchema>::Output::id_override()
    }
    fn make_linkable(schema: &mut Schema) -> Link {
        <Self as OverrideSchema>::Output::make_linkable(schema)
    }
    fn write_schema(schema: &mut Schema) -> Link {
        <Self as OverrideSchema>::Output::write_schema(schema)
    }
}
