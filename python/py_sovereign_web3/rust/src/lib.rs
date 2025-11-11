use pyo3::prelude::*;
use pyo3::exceptions::PyValueError;
use sovereign_web3::schema::{Serializer, UnsignedTransaction, Transaction, TransactionBuilder, UniquenessData};

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
        use sovereign_web3::schema::default_uniqueness;
        let uniqueness = default_uniqueness()
            .map_err(|e| PyValueError::new_err(format!("Failed to create default uniqueness: {}", e)))?;
        Ok(PyUniquenessData { inner: uniqueness })
    }
}

/// Python wrapper for TransactionBuilder
#[pyclass]
struct PyTransactionBuilder {
    inner: TransactionBuilder,
}

#[pymethods]
impl PyTransactionBuilder {
    /// Create a new transaction builder with a runtime call (JSON object)
    #[new]
    fn new(call_json: &str) -> PyResult<Self> {
        let call: serde_json::Value = serde_json::from_str(call_json)
            .map_err(|e| PyValueError::new_err(format!("Invalid JSON for runtime call: {}", e)))?;

        Ok(PyTransactionBuilder {
            inner: TransactionBuilder::new(call),
        })
    }

    /// Set priority fee in basis points
    fn priority_fee_bips(self, priority_fee_bips: u64) -> Self {
        PyTransactionBuilder {
            inner: self.inner.priority_fee_bips(priority_fee_bips),
        }
    }

    /// Set maximum fee
    fn max_fee(self, max_fee: u128) -> Self {
        PyTransactionBuilder {
            inner: self.inner.max_fee(max_fee),
        }
    }

    /// Set uniqueness data
    fn uniqueness(self, uniqueness: &PyUniquenessData) -> Self {
        PyTransactionBuilder {
            inner: self.inner.uniqueness(uniqueness.inner),
        }
    }

    /// Set gas limit (None for unlimited, Some(list) for specific limits)
    fn gas_limit(self, gas_limit: Option<Vec<u64>>) -> Self {
        PyTransactionBuilder {
            inner: self.inner.gas_limit(gas_limit),
        }
    }

    /// Set chain ID (required)
    fn chain_id(self, chain_id: u64) -> Self {
        PyTransactionBuilder {
            inner: self.inner.chain_id(chain_id),
        }
    }

    /// Build the unsigned transaction
    fn build(self) -> PyResult<PyUnsignedTransaction> {
        let unsigned_tx = self.inner.build()
            .map_err(|e| PyValueError::new_err(format!("Failed to build transaction: {}", e)))?;
        Ok(PyUnsignedTransaction { inner: unsigned_tx })
    }
}

/// A Python module implemented in Rust.
#[pymodule]
fn sovereign_web3(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySerializer>()?;
    m.add_class::<PyUnsignedTransaction>()?;
    m.add_class::<PyTransaction>()?;
    m.add_class::<PyUniquenessData>()?;
    m.add_class::<PyTransactionBuilder>()?;
    Ok(())
}
