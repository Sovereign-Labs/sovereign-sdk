use crate::common::Sequencer;
use crate::preferred::PreferredSequencer;
use crate::preferred::Runtime;
use crate::preferred::UniquenessData;
use borsh::BorshSerialize;
use derive_more::FromStr;
use sov_full_node_configs::sequencer::TimingOracleConfig;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::transaction::UnsignedTransaction;
use sov_modules_api::Amount;
use sov_modules_api::CryptoSpec;
use sov_modules_api::HexString;
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey;
use sov_modules_api::RawTx;
use sov_modules_api::Spec;
use sov_rollup_interface::node::da::DaService;
use tokio::sync::watch;
use tokio::time::Duration;

pub(crate) struct TimingOracleConfigWithPrivateKey<S: Spec> {
    time_oracle_config: TimingOracleConfig,
    priv_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
}

impl<S: Spec> TimingOracleConfigWithPrivateKey<S> {
    pub(crate) fn new(
        maybe_time_oracle_config: Option<TimingOracleConfig>,
    ) -> Option<anyhow::Result<Self>> {
        let time_oracle_config = maybe_time_oracle_config?;
        let priv_key = || -> anyhow::Result<_> {
            match &time_oracle_config.private_key_hex {
                Some(key_str) => {
                    let bytes = HexString::from_str(key_str).map_err(|_| anyhow::anyhow!("Invalid oracle private key hex - could not parse as hex. Check your preferred sequencer config file."))?.0;
                    let key = <S::CryptoSpec as CryptoSpec>::PrivateKey::try_from(bytes).map_err(|_| anyhow::anyhow!("Invalid oracle private key hex - invalid private key bytes. Check your preferred sequencer config file."))?;
                    Ok(key)
                }
                None => {
                    let key = <S::CryptoSpec as CryptoSpec>::PrivateKey::generate();
                    tracing::info!(
                        pub_key = %hex::encode(key.pub_key().as_ref()),
                        "Generated ephemeral oracle key. Make sure the paymaster is enabled.",

                    );
                    Ok(key)
                }
            }
        };

        Some(priv_key().map(|pk| Self {
            time_oracle_config,
            priv_key: pk,
        }))
    }

    fn priority_fee_bips(&self) -> PriorityFeeBips {
        PriorityFeeBips::from_percentage(self.time_oracle_config.priority_fee_percentage as u64)
    }

    fn max_fee(&self) -> Amount {
        Amount::new(self.time_oracle_config.max_fee.into())
    }

    fn interval(&self) -> Duration {
        Duration::from_millis(self.time_oracle_config.interval_millis)
    }

    pub(crate) fn address(&self) -> <S as Spec>::Address {
        self.priv_key.pub_key().credential_id().into()
    }
}

pub(crate) fn update_timestamp_task<S, Rt, Da>(
    seq: PreferredSequencer<S, Rt, Da>,
    oracle_config: TimingOracleConfigWithPrivateKey<S>,
    mut shutdown_receiver: watch::Receiver<()>,
) -> anyhow::Result<tokio::task::JoinHandle<()>>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    let runtime = Rt::default();

    let mut ticker = tokio::time::interval(oracle_config.interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut consecutive_failures = 0;

    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                 _ = ticker.tick() => {}
                 _ = shutdown_receiver.changed() => { break; }
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();

            let timestamp: i64 = now
                .as_millis()
                .try_into()
                .expect("Converting unix timestamp to i64 number of milliseconds failed");

            let message = Rt::maybe_set_oracle_timestamp(&runtime, timestamp).expect(
                "Oracle support must be checked before before spawning update_timestamp_task",
            );

            let details = TxDetails::<S> {
                max_priority_fee_bips: oracle_config.priority_fee_bips(),
                max_fee: oracle_config.max_fee(),
                gas_limit: None,
                chain_id: config_value!("CHAIN_ID"),
            };

            let unsigned_tx = UnsignedTransaction::<Rt, S>::new_with_details(
                message,
                UniquenessData::Generation(timestamp as u64),
                details,
            );

            let mut utx_bytes: Vec<u8> = Vec::new();
            BorshSerialize::serialize(&unsigned_tx, &mut utx_bytes).unwrap();
            utx_bytes.extend_from_slice(&Rt::CHAIN_HASH);

            let priv_key = &oracle_config.priv_key;

            let pub_key = priv_key.pub_key();
            let signature = priv_key.sign(&utx_bytes);

            let tx = unsigned_tx.to_signed_tx::<S::CryptoSpec>(pub_key, signature);
            let raw_tx = RawTx::new(borsh::to_vec(&tx).unwrap());

            let baked_tx = Rt::Auth::encode_with_standard_auth(raw_tx);

            if let Err(error) = seq.accept_tx(baked_tx).await {
                // Reduce log spam by only logging 1 of every 100 consecutive failures
                if consecutive_failures % 100 == 0 {
                    tracing::error!(?error, "Error submitting timestamp oracle update tx");
                }
                consecutive_failures += 1;
            } else {
                consecutive_failures = 0;
                tracing::trace!(%timestamp, "Successfully submitted timestamp oracle update tx");
            }
        }
    });

    Ok(handle)
}
