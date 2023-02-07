use std::marker::PhantomData;

use crate::{
    errors::CodecError,
    node_type::{LeafNode, Node, NodeKey, PhysicalLeafNode, PhysicalNode, PhysicalNodeKey},
    NodeBatch, PhysicalNodeBatch, Version,
};

/// `TreeReader` defines the interface between
/// [`JellyfishMerkleTree`](struct.JellyfishMerkleTree.html)
/// and underlying storage holding nodes.
pub trait TreeReader<K, H, const N: usize> {
    type Error: Into<anyhow::Error> + Send + Sync + 'static;
    /// Gets node given a node key. Returns error if the node does not exist.
    ///
    /// Recommended impl:
    /// ```ignore
    /// self.get_node_option(node_key)?.ok_or_else(|| Self::Error::from(format!("Missing node at {:?}.", node_key)))
    /// ```
    fn get_node(&self, node_key: &NodeKey<N>) -> Result<Node<K, H, N>, Self::Error>;

    /// Gets node given a node key. Returns `None` if the node does not exist.
    fn get_node_option(&self, node_key: &NodeKey<N>) -> Result<Option<Node<K, H, N>>, Self::Error>;

    /// Gets a value given a key. Returns `None` if the value does not exist.
    // TODO(@preston-evans98): Make the return type cheaply cloneable
    fn get_value(&self, key: &(Version, K)) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Gets the rightmost leaf at a version. Note that this assumes we are in the process of
    /// restoring the tree and all nodes are at the same version.
    fn get_rightmost_leaf(
        &self,
        version: Version,
    ) -> Result<Option<(NodeKey<N>, LeafNode<K, H, N>)>, Self::Error>;
}

pub trait PhysicalTreeReader<K> {
    type Error: Into<anyhow::Error> + Send + Sync + 'static;
    fn get_physical_node(&self, node_key: &PhysicalNodeKey)
        -> Result<PhysicalNode<K>, Self::Error>;

    fn get_physical_node_option(
        &self,
        node_key: &PhysicalNodeKey,
    ) -> Result<Option<PhysicalNode<K>>, Self::Error>;

    // TODO(@preston-evans98): Make the return type cheaply cloneable
    fn get_value(&self, key: &(Version, K)) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Gets the rightmost leaf at a version. Note that this assumes we are in the process of
    /// restoring the tree and all nodes are at the same version.
    fn get_rightmost_physical_leaf(
        &self,
        version: Version,
    ) -> Result<Option<(PhysicalNodeKey, PhysicalLeafNode<K>)>, Self::Error>;
}

pub trait TreeWriter<K, H, const N: usize>: Send + Sync {
    type Error: Into<anyhow::Error> + Send + Sync + 'static;
    fn write_node_batch(&self, node_batch: &NodeBatch<K, H, N>) -> Result<(), Self::Error>;
}

pub trait PhysicalTreeWriter<K>: Send + Sync {
    type Error: Into<anyhow::Error> + Send + Sync + 'static;
    fn write_physical_node_batch(
        &self,
        node_batch: &PhysicalNodeBatch<K>,
    ) -> Result<(), Self::Error>;
}

/// A typed wrapper around a byte-oriented data store, which automatically converts between
/// the raw on-disk tree nodes and the typed tree nodes consumed by the JMT
pub struct TypedStore<T, H, const N: usize> {
    pub inner: T,
    _phantom: PhantomData<H>,
}

impl<T, H, const N: usize> TypedStore<T, H, N> {
    pub fn new(reader: T) -> Self {
        Self {
            inner: reader,
            _phantom: PhantomData,
        }
    }
}

impl<T: PhysicalTreeReader<K, Error = E>, E, K, H, const N: usize> TreeReader<K, H, N>
    for TypedStore<T, H, N>
where
    E: Into<anyhow::Error> + From<CodecError> + Send + Sync + 'static,
{
    type Error = E;

    fn get_node(&self, node_key: &NodeKey<N>) -> Result<Node<K, H, N>, Self::Error> {
        let key = node_key.clone().into();
        let physical_node = self.inner.get_physical_node(&key)?;
        let node = physical_node.try_into()?;
        Ok(node)
    }

    fn get_node_option(&self, node_key: &NodeKey<N>) -> Result<Option<Node<K, H, N>>, Self::Error> {
        let key = node_key.clone().into();
        let physical_node_opt = self.inner.get_physical_node_option(&key)?;
        if let Some(physical_node) = physical_node_opt {
            let node = physical_node.try_into()?;
            return Ok(Some(node));
        }
        Ok(None)
    }

    fn get_value(&self, key: &(Version, K)) -> Result<Option<Vec<u8>>, Self::Error> {
        self.inner.get_value(key)
    }

    fn get_rightmost_leaf(
        &self,
        _version: Version,
    ) -> Result<Option<(NodeKey<N>, LeafNode<K, H, N>)>, Self::Error> {
        todo!()
    }
}

impl<T: PhysicalTreeWriter<K, Error = E>, E, K: Clone, H: Clone + Send + Sync, const N: usize>
    TreeWriter<K, H, N> for TypedStore<T, H, N>
where
    E: Into<anyhow::Error> + From<CodecError> + Send + Sync + 'static,
{
    type Error = E;

    fn write_node_batch(&self, node_batch: &NodeBatch<K, H, N>) -> Result<(), Self::Error> {
        let physical_node_batch = node_batch
            .iter()
            .map(|(k, v)| (k.clone().into(), v.clone().into()))
            .collect();
        self.inner.write_physical_node_batch(&physical_node_batch)
    }
}
