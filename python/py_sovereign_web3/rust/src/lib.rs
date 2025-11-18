use pyo3::exceptions::PyValueError;
use pyo3::types::PyDict;
use pyo3::{prelude::*, types::PyType};
use sovereign_web3::schema::{
    default_uniqueness, Serializer, Transaction, TxDetails, UniquenessData, UnsignedTransaction,
    DEFAULT_MAX_FEE, DEFAULT_MAX_PRIORITY_FEE_BIPS,
};

#[pyclass(name = "Serializer")]
struct PySerializer {
    inner: Serializer,
}

#[pymethods]
impl PySerializer {
    #[new]
    fn new(schema_json: &str) -> PyResult<Self> {
        let serializer = Serializer::from_json(schema_json)
            .map_err(|e| PyValueError::new_err(format!("Failed to create serializer: {}", e)))?;
        Ok(PySerializer { inner: serializer })
    }

    #[classmethod]
    fn from_url(_cls: &Bound<'_, PyType>, url: &str) -> PyResult<Self> {
        let serializer = Serializer::from_url(url).map_err(|e| {
            PyValueError::new_err(format!("Failed to fetch schema from URL: {}", e))
        })?;
        Ok(PySerializer { inner: serializer })
    }

    fn chain_hash(&self) -> PyResult<Vec<u8>> {
        let hash = self
            .inner
            .chain_hash()
            .map_err(|e| PyValueError::new_err(format!("Failed to get chain hash: {}", e)))?;
        Ok(hash.to_vec())
    }

    fn serialize_unsigned_tx(&self, unsigned_tx: &PyUnsignedTransaction) -> PyResult<Vec<u8>> {
        let bytes = self
            .inner
            .serialize_unsigned_tx(&unsigned_tx.inner)
            .map_err(|e| {
                PyValueError::new_err(format!("Failed to serialize unsigned transaction: {}", e))
            })?;
        Ok(bytes)
    }

    fn serialize_tx(&self, tx: &PyTransaction) -> PyResult<Vec<u8>> {
        let bytes = self.inner.serialize_tx(&tx.inner).map_err(|e| {
            PyValueError::new_err(format!("Failed to serialize transaction: {}", e))
        })?;
        Ok(bytes)
    }
}

#[pyclass(name = "TxDetails")]
struct PyTxDetails {
    inner: TxDetails,
}

#[pymethods]
impl PyTxDetails {
    #[new]
    #[pyo3(signature = (chain_id, max_fee=DEFAULT_MAX_FEE, max_priority_fee_bips=DEFAULT_MAX_PRIORITY_FEE_BIPS, gas_limit=None))]
    fn new(
        chain_id: u64,
        max_fee: u128,
        max_priority_fee_bips: u64,
        gas_limit: Option<Vec<u64>>,
    ) -> Self {
        PyTxDetails {
            inner: TxDetails {
                max_priority_fee_bips,
                max_fee,
                gas_limit,
                chain_id,
            },
        }
    }

    #[getter]
    fn get_chain_id(&self) -> u64 {
        self.inner.chain_id
    }

    #[setter]
    fn set_chain_id(&mut self, chain_id: u64) {
        self.inner.chain_id = chain_id;
    }

    #[getter]
    fn get_max_fee(&self) -> u128 {
        self.inner.max_fee
    }

    #[setter]
    fn set_max_fee(&mut self, max_fee: u128) {
        self.inner.max_fee = max_fee;
    }

    #[getter]
    fn get_max_priority_fee_bips(&self) -> u64 {
        self.inner.max_priority_fee_bips
    }

    #[setter]
    fn set_max_priority_fee_bips(&mut self, max_priority_fee_bips: u64) {
        self.inner.max_priority_fee_bips = max_priority_fee_bips;
    }
}

#[pyclass(name = "UnsignedTransaction")]
struct PyUnsignedTransaction {
    inner: UnsignedTransaction,
}

#[pymethods]
impl PyUnsignedTransaction {
    #[new]
    #[pyo3(signature = (runtime_call, details, uniqueness=None))]
    fn new(
        runtime_call: &Bound<'_, PyDict>,
        details: &PyTxDetails,
        uniqueness: Option<&PyUniquenessData>,
    ) -> PyResult<Self> {
        let call: serde_json::Value = pythonize::depythonize(runtime_call).map_err(|e| {
            PyValueError::new_err(format!(
                "Failed to convert runtime_call dict to JSON: {}",
                e
            ))
        })?;

        let uniqueness = match uniqueness {
            Some(u) => u.inner,
            None => default_uniqueness().map_err(|e| {
                PyValueError::new_err(format!("Failed to create default uniqueness: {}", e))
            })?,
        };

        let unsigned_tx = UnsignedTransaction {
            runtime_call: call,
            uniqueness,
            details: details.inner.clone(),
        };

        Ok(PyUnsignedTransaction { inner: unsigned_tx })
    }

    fn bytes_for_signing(&self, serializer: &PySerializer) -> PyResult<Vec<u8>> {
        let bytes = self
            .inner
            .bytes_for_signing(&serializer.inner)
            .map_err(|e| PyValueError::new_err(format!("Failed to get signing bytes: {}", e)))?;
        Ok(bytes)
    }

    fn to_signed(&self, pub_key: Vec<u8>, signature: Vec<u8>) -> PyTransaction {
        let signed = self.inner.to_signed(pub_key, signature);
        PyTransaction { inner: signed }
    }
}

#[pyclass(name = "Transaction")]
struct PyTransaction {
    inner: Transaction,
}

#[pyclass(name = "UniquenessData")]
#[derive(Clone)]
struct PyUniquenessData {
    inner: UniquenessData,
}

#[pymethods]
impl PyUniquenessData {
    #[staticmethod]
    fn nonce(nonce: u64) -> Self {
        PyUniquenessData {
            inner: UniquenessData::Nonce(nonce),
        }
    }

    #[staticmethod]
    fn generation(generation: u64) -> Self {
        PyUniquenessData {
            inner: UniquenessData::Generation(generation),
        }
    }

    #[staticmethod]
    fn default() -> PyResult<Self> {
        let uniqueness = default_uniqueness().map_err(|e| {
            PyValueError::new_err(format!("Failed to create default uniqueness: {}", e))
        })?;
        Ok(PyUniquenessData { inner: uniqueness })
    }
}

#[pymodule(name = "sovereign_web3")]
mod py_sovereign_web3 {
    #[pymodule_export]
    use super::{
        PySerializer, PyTransaction, PyTxDetails, PyUniquenessData, PyUnsignedTransaction,
    };
}
