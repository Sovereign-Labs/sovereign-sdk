use pyo3::types::PyDict;
use pyo3::{prelude::*, types::PyType};
use pyo3::exceptions::PyValueError;
use ::sovereign_web3::schema::{Serializer, UnsignedTransaction, Transaction, TransactionBuilder, UniquenessData};

/// Python wrapper for the Rust Serializer struct
#[pyclass]
struct PySerializer {
    inner: Serializer,
}

#[pymethods]
impl PySerializer {
    /// Create a new serializer from a JSON schema string
    #[new]
    fn new(schema_json: &str) -> PyResult<Self> {
        let serializer = Serializer::from_json(schema_json)
            .map_err(|e| PyValueError::new_err(format!("Failed to create serializer: {}", e)))?;
        Ok(PySerializer { inner: serializer })
    }

    /// Create a serializer by fetching schema from a URL
    #[classmethod]
    fn from_url(_cls: &Bound<'_, PyType>, url: &str) -> PyResult<Self> {
        let serializer = Serializer::from_url(url)
            .map_err(|e| PyValueError::new_err(format!("Failed to fetch schema from URL: {}", e)))?;
        Ok(PySerializer { inner: serializer })
    }

    /// Get the 32-byte chain hash from the schema
    fn chain_hash(&self) -> PyResult<Vec<u8>> {
        let hash = self.inner.chain_hash()
            .map_err(|e| PyValueError::new_err(format!("Failed to get chain hash: {}", e)))?;
        Ok(hash.to_vec())
    }

    /// Serialize an unsigned transaction to binary format
    fn serialize_unsigned_tx(&self, unsigned_tx: &PyUnsignedTransaction) -> PyResult<Vec<u8>> {
        let bytes = self.inner.serialize_unsigned_tx(&unsigned_tx.inner)
            .map_err(|e| PyValueError::new_err(format!("Failed to serialize unsigned transaction: {}", e)))?;
        Ok(bytes)
    }

    /// Serialize a signed transaction to binary format
    fn serialize_tx(&self, tx: &PyTransaction) -> PyResult<Vec<u8>> {
        let bytes = self.inner.serialize_tx(&tx.inner)
            .map_err(|e| PyValueError::new_err(format!("Failed to serialize transaction: {}", e)))?;
        Ok(bytes)
    }
}

/// Python wrapper for UnsignedTransaction
#[pyclass]
struct PyUnsignedTransaction {
    inner: UnsignedTransaction,
}

#[pymethods]
impl PyUnsignedTransaction {
    /// Create a new unsigned transaction
    #[new]
    #[pyo3(signature = (runtime_call, chain_id=None, uniqueness=None, max_fee=None, max_priority_fee_bips=None, gas_limit=None))]
    fn new(
        runtime_call: &Bound<'_, PyDict>,
        chain_id: Option<u64>,
        uniqueness: Option<&PyUniquenessData>,
        max_fee: Option<u128>,
        max_priority_fee_bips: Option<u64>,
        gas_limit: Option<Vec<u64>>,
    ) -> PyResult<Self> {
        use ::sovereign_web3::schema::{default_uniqueness, TxDetails, DEFAULT_MAX_FEE, DEFAULT_MAX_PRIORITY_FEE_BIPS};

        // Convert Python dict to JSON Value
        let call: serde_json::Value = pythonize::depythonize_bound(runtime_call.clone())
            .map_err(|e| PyValueError::new_err(format!("Failed to convert runtime_call to JSON: {}", e)))?;

        // Use provided uniqueness or default
        let uniqueness = match uniqueness {
            Some(u) => u.inner,
            None => default_uniqueness()
                .map_err(|e| PyValueError::new_err(format!("Failed to create default uniqueness: {}", e)))?,
        };

        // Create transaction details with defaults
        let details = TxDetails {
            max_priority_fee_bips: max_priority_fee_bips.unwrap_or(DEFAULT_MAX_PRIORITY_FEE_BIPS),
            max_fee: max_fee.unwrap_or(DEFAULT_MAX_FEE),
            gas_limit: Some(gas_limit).flatten(),
            chain_id: chain_id.unwrap(), // todo: raise exception
        };

        let unsigned_tx = UnsignedTransaction {
            runtime_call: call,
            uniqueness,
            details,
        };

        Ok(PyUnsignedTransaction { inner: unsigned_tx })
    }

    /// Get bytes that should be signed (includes chain hash)
    fn bytes_for_signing(&self, serializer: &PySerializer) -> PyResult<Vec<u8>> {
        let bytes = self.inner.bytes_for_signing(&serializer.inner)
            .map_err(|e| PyValueError::new_err(format!("Failed to get signing bytes: {}", e)))?;
        Ok(bytes)
    }

    /// Convert to a signed transaction with the provided public key and signature
    fn to_signed(&self, pub_key: Vec<u8>, signature: Vec<u8>) -> PyTransaction {
        let signed = self.inner.to_signed(pub_key, signature);
        PyTransaction { inner: signed }
    }
}

/// Python wrapper for Transaction
#[pyclass]
struct PyTransaction {
    inner: Transaction,
}

/// Python wrapper for UniquenessData
#[pyclass]
#[derive(Clone)]
struct PyUniquenessData {
    inner: UniquenessData,
}

#[pymethods]
impl PyUniquenessData {
    /// Create nonce-based uniqueness
    #[staticmethod]
    fn nonce(nonce: u64) -> Self {
        PyUniquenessData {
            inner: UniquenessData::Nonce(nonce),
        }
    }

    /// Create generation-based uniqueness
    #[staticmethod]
    fn generation(generation: u64) -> Self {
        PyUniquenessData {
            inner: UniquenessData::Generation(generation),
        }
    }

    /// Create default uniqueness (timestamp-based)
    #[staticmethod]
    fn default() -> PyResult<Self> {
        use ::sovereign_web3::schema::default_uniqueness;
        let uniqueness = default_uniqueness()
            .map_err(|e| PyValueError::new_err(format!("Failed to create default uniqueness: {}", e)))?;
        Ok(PyUniquenessData { inner: uniqueness })
    }
}

/// A Python module implemented in Rust.
#[pymodule]
fn sovereign_web3(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySerializer>()?;
    m.add_class::<PyUnsignedTransaction>()?;
    m.add_class::<PyTransaction>()?;
    m.add_class::<PyUniquenessData>()?;
    Ok(())
}
