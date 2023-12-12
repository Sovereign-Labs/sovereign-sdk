use std::{fmt, io, marker};

use serde::ser::SerializeStruct;
#[cfg(feature = "native")]
use sov_modules_core::PrivateKey;
use sov_modules_core::{Context, Signature};
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_macros::cycle_tracker;

const EXTEND_MESSAGE_LEN: usize = 3 * core::mem::size_of::<u64>();

/// A Transaction object that is compatible with the module-system/sov-default-stf.
#[derive(
    Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize, serde::Serialize,
)]
pub struct Transaction<C: Context> {
    signature: C::Signature,
    pub_key: C::PublicKey,
    runtime_msg: Vec<u8>,
    chain_id: u64,
    gas_tip: u64,
    nonce: u64,
}

/// An unsent transaction with the required data to be submitted to the DA layer
#[derive(Debug)]
pub struct UnsignedTransaction<Tx> {
    /// The underlying transaction
    pub tx: Tx,
    /// The ID of the target chain
    pub chain_id: u64,
    /// The gas tip for the sequencer
    pub gas_tip: u64,
}

impl<C: Context> Transaction<C> {
    pub fn signature(&self) -> &C::Signature {
        &self.signature
    }

    pub fn pub_key(&self) -> &C::PublicKey {
        &self.pub_key
    }

    pub fn runtime_msg(&self) -> &[u8] {
        &self.runtime_msg
    }

    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub const fn gas_tip(&self) -> u64 {
        self.gas_tip
    }

    /// Check whether the transaction has been signed correctly.
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    pub fn verify(&self) -> anyhow::Result<()> {
        let mut serialized_tx = Vec::with_capacity(self.runtime_msg().len() + EXTEND_MESSAGE_LEN);

        serialized_tx.extend_from_slice(self.runtime_msg());
        serialized_tx.extend_from_slice(&self.chain_id().to_le_bytes());
        serialized_tx.extend_from_slice(&self.gas_tip().to_le_bytes());
        serialized_tx.extend_from_slice(&self.nonce().to_le_bytes());

        self.signature().verify(&self.pub_key, &serialized_tx)?;

        Ok(())
    }

    /// New transaction.
    pub fn new(
        pub_key: C::PublicKey,
        message: Vec<u8>,
        signature: C::Signature,
        chain_id: u64,
        gas_tip: u64,
        nonce: u64,
    ) -> Self {
        Self {
            signature,
            runtime_msg: message,
            pub_key,
            chain_id,
            gas_tip,
            nonce,
        }
    }
}

#[cfg(feature = "native")]
impl<C: Context> Transaction<C> {
    /// New signed transaction.
    pub fn new_signed_tx(
        priv_key: &C::PrivateKey,
        mut message: Vec<u8>,
        chain_id: u64,
        gas_tip: u64,
        nonce: u64,
    ) -> Self {
        // Since we own the message already, try to add the serialized nonce in-place.
        // This lets us avoid a copy if the message vec has at least 8 bytes of extra capacity.
        let len = message.len();

        // resizes once to avoid potential multiple realloc
        message.resize(len + EXTEND_MESSAGE_LEN, 0);

        message[len..len + 8].copy_from_slice(&chain_id.to_le_bytes());
        message[len + 8..len + 16].copy_from_slice(&gas_tip.to_le_bytes());
        message[len + 16..len + 24].copy_from_slice(&nonce.to_le_bytes());

        let pub_key = priv_key.pub_key();
        let signature = priv_key.sign(&message);

        // Don't forget to truncate the message back to its original length!
        message.truncate(len);

        Self {
            signature,
            runtime_msg: message,
            pub_key,
            chain_id,
            gas_tip,
            nonce,
        }
    }
}

impl<Tx> UnsignedTransaction<Tx> {
    pub const fn new(tx: Tx, chain_id: u64, gas_tip: u64) -> Self {
        Self {
            tx,
            chain_id,
            gas_tip,
        }
    }
}

impl<Tx> borsh::BorshSerialize for UnsignedTransaction<Tx>
where
    Tx: borsh::BorshSerialize,
{
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        self.tx.serialize(writer)?;
        self.chain_id.serialize(writer)?;
        self.gas_tip.serialize(writer)?;

        Ok(())
    }
}

impl<Tx> borsh::BorshDeserialize for UnsignedTransaction<Tx>
where
    Tx: borsh::BorshDeserialize,
{
    fn deserialize_reader<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let tx = Tx::deserialize_reader(reader)?;
        let chain_id = u64::deserialize_reader(reader)?;
        let gas_tip = u64::deserialize_reader(reader)?;

        Ok(Self {
            tx,
            chain_id,
            gas_tip,
        })
    }
}

impl<Tx> serde::Serialize for UnsignedTransaction<Tx>
where
    Tx: serde::Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("UnsignedTransaction", 3)?;
        state.serialize_field("tx", &self.tx)?;
        state.serialize_field("chain_id", &self.chain_id)?;
        state.serialize_field("gas_tip", &self.gas_tip)?;
        state.end()
    }
}

impl<'de, Tx> serde::Deserialize<'de> for UnsignedTransaction<Tx>
where
    Tx: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        #[derive(serde::Deserialize)]
        #[serde(field_identifier)]
        #[allow(non_camel_case_types)]
        enum Field {
            tx,
            chain_id,
            gas_tip,
        }

        struct UnsignedTransactionVisitor<Tx>(marker::PhantomData<Tx>);

        impl<'de, Tx> de::Visitor<'de> for UnsignedTransactionVisitor<Tx>
        where
            Tx: serde::Deserialize<'de>,
        {
            type Value = UnsignedTransaction<Tx>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct UnsignedTransaction")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<UnsignedTransaction<Tx>, V::Error>
            where
                V: de::SeqAccess<'de>,
            {
                let tx = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let chain_id = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let gas_tip = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;

                Ok(UnsignedTransaction::new(tx, chain_id, gas_tip))
            }

            fn visit_map<V>(self, mut map: V) -> Result<UnsignedTransaction<Tx>, V::Error>
            where
                V: de::MapAccess<'de>,
            {
                let mut tx = None;
                let mut chain_id = None;
                let mut gas_tip = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::tx if tx.is_some() => return Err(de::Error::duplicate_field("tx")),
                        Field::tx => tx = Some(map.next_value()?),
                        Field::chain_id if chain_id.is_some() => {
                            return Err(de::Error::duplicate_field("chain_id"))
                        }
                        Field::chain_id => chain_id = Some(map.next_value()?),
                        Field::gas_tip if gas_tip.is_some() => {
                            return Err(de::Error::duplicate_field("gas_tip"))
                        }
                        Field::gas_tip => gas_tip = Some(map.next_value()?),
                    }
                }

                let tx = tx.ok_or_else(|| de::Error::missing_field("tx"))?;
                let chain_id = chain_id.ok_or_else(|| de::Error::missing_field("chain_id"))?;
                let gas_tip = gas_tip.ok_or_else(|| de::Error::missing_field("gas_tip"))?;

                Ok(UnsignedTransaction::new(tx, chain_id, gas_tip))
            }
        }

        const FIELDS: &[&str] = &["tx", "chain_id", "gas_tip"];
        deserializer.deserialize_struct(
            "UnsignedTransaction",
            FIELDS,
            UnsignedTransactionVisitor::<Tx>(marker::PhantomData),
        )
    }
}
