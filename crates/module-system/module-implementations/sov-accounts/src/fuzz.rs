use arbitrary::{Arbitrary, Unstructured};
use sov_modules_api::{CryptoSpec, DaSpec, Module, SafeVec, Spec, StateCheckpoint};

use crate::{Account, AccountConfig, AccountData, Accounts, CallMessage};

impl<'a> Arbitrary<'a> for CallMessage {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::InsertCredentialId(u.arbitrary()?))
    }
}

impl<'a> Arbitrary<'a> for Account {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let mut allowed_credentials = SafeVec::new();
        let iter = u.arbitrary_iter()?;
        for credential_id in iter {
            let credential_id = credential_id?;
            if allowed_credentials.try_push(credential_id).is_err() {
                return Ok(Self {
                    allowed_credentials,
                });
            }
        }

        Ok(Self {
            allowed_credentials,
        })
    }
}

impl<'a, Addr: arbitrary::Arbitrary<'a>> Arbitrary<'a> for AccountData<Addr> {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            credential_id: u.arbitrary()?,
            address: u.arbitrary()?,
        })
    }
}

impl<'a, S> Arbitrary<'a> for AccountConfig<S>
where
    S: Spec,
    S::Address: Arbitrary<'a>,
    <S::CryptoSpec as CryptoSpec>::PublicKey: Arbitrary<'a>,
{
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            accounts: u.arbitrary_iter()?.collect::<Result<_, _>>()?,
            enable_custom_account_mappings: u.arbitrary()?,
        })
    }
}

impl<'a, S> Accounts<S>
where
    S: Spec,
    S::Address: Arbitrary<'a>,
    <S::Da as DaSpec>::BlockHeader: Default,
    <S::CryptoSpec as CryptoSpec>::PublicKey: Arbitrary<'a>,
{
    /// Creates an arbitrary set of accounts and stores it under `state`.
    pub fn arbitrary_workset(
        u: &mut Unstructured<'a>,
        state: &mut StateCheckpoint<S>,
    ) -> arbitrary::Result<Self> {
        let config: AccountConfig<S> = u.arbitrary()?;
        let mut accounts = Accounts::default();
        let mut genesis_state = state.to_genesis_state_accessor::<Accounts<S>>(&config);

        if accounts
            .genesis(&Default::default(), &config, &mut genesis_state)
            .is_err()
        {
            return Err(arbitrary::Error::IncorrectFormat);
        };

        Ok(accounts)
    }
}
