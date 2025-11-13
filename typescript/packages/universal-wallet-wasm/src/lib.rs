use sov_universal_wallet::schema::{RollupRoots, Schema as NativeSchema};
use wasm_bindgen::prelude::*;

trait MapErrorToJs<T> {
    fn map_err_to_js(self) -> Result<T, JsError>;
}

impl<T, E> MapErrorToJs<T> for Result<T, E>
where
    E: ToString,
{
    fn map_err_to_js(self) -> Result<T, JsError> {
        self.map_err(|e| JsError::new(&e.to_string()))
    }
}

/// Well-known ids of types utilized in the rollup.
#[wasm_bindgen]
pub enum KnownTypeId {
    /// The type id of the transaction.
    Transaction = 0,
    /// The type id of the unsigned transaction.
    UnsignedTransaction = 1,
    /// The type id of the runtime call.
    RuntimeCall = 2,
}

impl From<KnownTypeId> for RollupRoots {
    fn from(value: KnownTypeId) -> Self {
        match value {
            KnownTypeId::Transaction => RollupRoots::Transaction,
            KnownTypeId::UnsignedTransaction => RollupRoots::UnsignedTransaction,
            KnownTypeId::RuntimeCall => RollupRoots::RuntimeCall,
        }
    }
}

/// A rollup schema is a description of the types that are utilized in the rollup.
/// It is used to serialize and deserialize the types.
/// As well as display them in a human-readable way that is verified by the rollup.
#[wasm_bindgen]
pub struct Schema {
    json: String,
    inner: NativeSchema,
}

#[wasm_bindgen]
impl Schema {
    /// Creates a `Schema` instance from the provided JSON descriptor.
    #[wasm_bindgen(js_name = fromJSON)]
    pub fn from_json(json: &str) -> Result<Schema, JsError> {
        Ok(Schema {
            json: json.to_string(),
            inner: NativeSchema::from_json(json).map_err_to_js()?,
        })
    }

    /// Returns the JSON descriptor that was used to create this schema instance.
    #[wasm_bindgen(getter)]
    pub fn descriptor(&self) -> String {
        self.json.clone()
    }

    /// Converts the provided JSON to borsh according to the provided schema.
    #[wasm_bindgen(js_name = jsonToBorsh)]
    pub fn json_to_borsh(&self, type_index: usize, input: &str) -> Result<Vec<u8>, JsError> {
        self.inner.json_to_borsh(type_index, input).map_err_to_js()
    }

    /// Displays the provided borsh bytes as a string according to the provided schema.
    pub fn display(&self, type_index: usize, input: &[u8]) -> Result<String, JsError> {
        self.inner.display(type_index, input).map_err_to_js()
    }

    /// Get the index of the provided known type within the schema.
    #[wasm_bindgen(js_name = knownTypeIndex)]
    pub fn known_type_index(&self, known_type_id: KnownTypeId) -> Result<usize, JsError> {
        self.inner
            .rollup_expected_index(known_type_id.into())
            .map_err_to_js()
    }

    /// Get the metadata hash from the schema.
    #[wasm_bindgen(getter, js_name = metadataHash)]
    pub fn metadata_hash(&self) -> Result<Vec<u8>, JsError> {
        Ok(self.inner.metadata_hash().map_err_to_js()?.to_vec())
    }

    /// Get the chain hash from the schema.
    #[wasm_bindgen(getter, js_name = chainHash)]
    pub fn chain_hash(&self) -> Result<Vec<u8>, JsError> {
        Ok(self.inner.chain_hash().map_err_to_js()?.to_vec())
    }

    #[wasm_bindgen(js_name = eip712Json)]
    pub fn eip712_json(&self, type_index: usize, input: &[u8]) -> Result<String, JsError> {
        self.inner.eip712_json(type_index, input).map_err_to_js()
    }

    #[wasm_bindgen(js_name = eip712SigningHash)]
    pub fn eip712_signing_hash(&self, type_index: usize, input: &[u8]) -> Result<Vec<u8>, JsError> {
        Ok(self.inner.eip712_signing_hash(type_index, input).map_err_to_js()?.to_vec())
    }
}
