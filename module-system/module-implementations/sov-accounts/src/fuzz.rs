use arbitrary::{Arbitrary, Unstructured};
use proptest::arbitrary::any;
use proptest::strategy::{BoxedStrategy, Strategy};
use sov_modules_api::{Context, Module, PrivateKey, WorkingSet};

use crate::{Account, AccountConfig, Accounts, CallMessage};

impl<'a, C> Arbitrary<'a> for CallMessage<C>
where
    C: Context,
    C::PrivateKey: Arbitrary<'a>,
{
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let secret = C::PrivateKey::arbitrary(u)?;
        let public = secret.pub_key();

        let payload_len = u.arbitrary_len::<u8>()?;
        let payload = u.bytes(payload_len)?;
        let signature = secret.sign(payload);

        Ok(Self::UpdatePublicKey(public, signature))
    }
}

impl<C> proptest::arbitrary::Arbitrary for CallMessage<C>
where
    C: Context,
    C::PrivateKey: proptest::arbitrary::Arbitrary,
{
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        (any::<C::PrivateKey>(), any::<Vec<u8>>())
            .prop_map(|(secret, payload)| {
                let public = secret.pub_key();
                let signature = secret.sign(&payload);
                Self::UpdatePublicKey(public, signature)
            })
            .boxed()
    }
}

impl<'a, C> Arbitrary<'a> for Account<C>
where
    C: Context,
    C::Address: Arbitrary<'a>,
{
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let addr = u.arbitrary()?;
        let nonce = u.arbitrary()?;
        Ok(Self { addr, nonce })
    }
}

impl<C> proptest::arbitrary::Arbitrary for Account<C>
where
    C: Context,
    C::Address: proptest::arbitrary::Arbitrary,
{
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        (any::<C::Address>(), any::<u64>())
            .prop_map(|(addr, nonce)| Account { addr, nonce })
            .boxed()
    }
}

impl<'a, C> Arbitrary<'a> for AccountConfig<C>
where
    C: Context,
    C::PublicKey: Arbitrary<'a>,
{
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // TODO we might want a dedicated struct that will generate the private key counterpart so
        // payloads can be signed and verified
        Ok(Self {
            pub_keys: u.arbitrary_iter()?.collect::<Result<_, _>>()?,
        })
    }
}

impl<C> proptest::arbitrary::Arbitrary for AccountConfig<C>
where
    C: Context,
    C::PrivateKey: proptest::arbitrary::Arbitrary,
{
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        any::<Vec<C::PrivateKey>>()
            .prop_map(|keys| AccountConfig {
                pub_keys: keys.into_iter().map(|k| k.pub_key()).collect(),
            })
            .boxed()
    }
}

impl<'a, C> Accounts<C>
where
    C: Context,
    C::Address: Arbitrary<'a>,
    C::PublicKey: Arbitrary<'a>,
{
    /// Creates an arbitrary set of accounts and stores it under `working_set`.
    pub fn arbitrary_workset(
        u: &mut Unstructured<'a>,
        working_set: &mut WorkingSet<C>,
    ) -> arbitrary::Result<Self> {
        let config: AccountConfig<C> = u.arbitrary()?;
        let accounts = Accounts::default();

        accounts
            .genesis(&config, working_set)
            .map_err(|_| arbitrary::Error::IncorrectFormat)?;

        Ok(accounts)
    }
}
