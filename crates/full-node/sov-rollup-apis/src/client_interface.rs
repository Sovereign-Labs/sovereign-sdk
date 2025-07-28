use std::num::ParseIntError;

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::types;
use sov_modules_api::prelude::anyhow;
use sov_modules_api::prelude::anyhow::Context;
use sov_modules_api::transaction::{PriorityFeeBips, TransactionConsumption, TxDetails};
use sov_modules_api::{Amount, Gas, Spec, StoredEvent, TransactionReceipt, TxEffect, *};
use sov_modules_stf_blueprint::ApplyTxResult;
use sov_rollup_interface::common::HexString;

use crate::SimulateExecutionContainer;

fn decode_b64(data: &str) -> anyhow::Result<Vec<u8>> {
    BASE64_STANDARD
        .decode(data)
        .context("Failed to decode base64 data in aggregated proof response")
}

/// To build a [`SimulateExecutionContainer`] from [`types::SimulateExecutionResponse`], we need to decode the
/// base64 encoded data in the response.
/// Note that we don't have a way to get the raw transaction hash because it depends on the signature, so it is
/// replaced by a dummy value in the [`SimulateExecutionContainer`].
impl<S: Spec> TryFrom<types::SimulateExecutionResponse> for SimulateExecutionContainer<S> {
    type Error = anyhow::Error;

    fn try_from(value: types::SimulateExecutionResponse) -> Result<Self, Self::Error> {
        let value = value.apply_tx_result;
        let received_consumption = value.transaction_consumption;

        let remaining_funds = received_consumption
            .remaining_funds
            .as_str()
            .parse::<Amount>()?;

        let gas_price = received_consumption
            .gas_price
            .0
            .iter()
            .map(|item| item.as_str().parse::<Amount>())
            .collect::<Result<Vec<_>, _>>()?;

        let transaction_consumption = TransactionConsumption::<S::Gas>::new(
            remaining_funds,
            S::Gas::try_from(received_consumption.base_fee.0).map_err(Into::into)?,
            received_consumption
                .priority_fee
                .as_str()
                .parse::<Amount>()?,
            <S::Gas as Gas>::Price::try_from(gas_price).map_err(Into::into)?,
        );

        let received_receipt = value.receipt;

        let events = {
            let mut events = Vec::with_capacity(received_receipt.events.len());
            for types::StoredEvent {
                key: event_key,
                value: event_value,
            } in received_receipt.events
            {
                let decoded_key = decode_b64(&event_key)?;
                let decoded_value = decode_b64(&event_value)?;
                events.push(StoredEvent::new(&decoded_key, &decoded_value, [0; 32]));
            }

            events
        };

        let receipt = {
            let effect = received_receipt.receipt;
            let inner = effect.content;

            match effect.outcome {
                types::TxEffectOutcome::Successful => {
                    let decoded_inner: SuccessfulTxContents<S> =
                        serde_json::from_slice(&decode_b64(&inner)?)?;
                    TxEffect::Successful(decoded_inner)
                }
                types::TxEffectOutcome::Reverted => {
                    let decoded_inner: RevertedTxContents<S> =
                        serde_json::from_slice(&decode_b64(&inner)?)?;
                    TxEffect::Reverted(decoded_inner)
                }
                types::TxEffectOutcome::Skipped => {
                    let decoded_inner: SkippedTxContents<S> =
                        serde_json::from_slice(&decode_b64(&inner)?)?;
                    TxEffect::Skipped(decoded_inner)
                }
            }
        };

        let receipt = TransactionReceipt {
            tx_hash: HexString::new([0; 32]),
            body_to_save: None,
            events,
            receipt,
        };

        Ok(Self {
            apply_tx_result: ApplyTxResult {
                transaction_consumption,
                receipt,
            },
        })
    }
}

impl<S: Spec> TryInto<types::SimulateExecutionResponse> for SimulateExecutionContainer<S> {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<types::SimulateExecutionResponse, Self::Error> {
        let result = self.apply_tx_result;
        let transaction_consumption = result.transaction_consumption;

        let remaining_funds = transaction_consumption.remaining_funds().0;
        let base_fee = types::GasUnit(transaction_consumption.base_fee().as_ref().to_vec());
        let priority_fee = transaction_consumption.priority_fee().0;
        let gas_price = types::GasPrice(
            transaction_consumption
                .gas_price()
                .as_ref()
                .iter()
                .map(|item| {
                    item.to_string()
                        .try_into()
                        .expect("Api spec rejected valid integer. This should never happen.")
                })
                .collect::<Vec<types::GasPriceItem>>(),
        );

        let transaction_consumption = types::TransactionConsumption {
            remaining_funds: types::TransactionConsumptionRemainingFunds::try_from(
                remaining_funds.to_string(),
            )
            .expect("Failed to convert remaining funds to string"),
            base_fee,
            priority_fee: types::TransactionConsumptionPriorityFee::try_from(
                priority_fee.to_string(),
            )
            .expect("Failed to convert priority fee to string"),
            gas_price,
        };

        let receipt = result.receipt;

        let events = receipt
            .events
            .into_iter()
            .map(|event| types::StoredEvent {
                key: BASE64_STANDARD.encode(event.key().inner()),
                value: BASE64_STANDARD.encode(event.value().inner()),
            })
            .collect();

        let effect = match receipt.receipt {
            TxEffect::Successful(content) => types::TxEffect {
                outcome: types::TxEffectOutcome::Successful,
                content: BASE64_STANDARD.encode(serde_json::to_vec(&content)?),
            },
            TxEffect::Reverted(content) => types::TxEffect {
                outcome: types::TxEffectOutcome::Reverted,
                content: BASE64_STANDARD.encode(serde_json::to_vec(&content)?),
            },
            TxEffect::Skipped(reason) => types::TxEffect {
                outcome: types::TxEffectOutcome::Skipped,
                content: BASE64_STANDARD.encode(serde_json::to_vec(&reason)?),
            },
        };

        let receipt = types::TxReceiptRollup {
            events,
            receipt: effect,
        };

        Ok(types::SimulateExecutionResponse {
            apply_tx_result: types::ApplyTxResult {
                transaction_consumption,
                receipt,
            },
        })
    }
}

impl<S: Spec> TryInto<types::PartialTransaction> for crate::PartialTransaction<S> {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<types::PartialTransaction, Self::Error> {
        let sender_pub_key = serde_json::to_string(&self.sender_pub_key)?;
        let encoded_call_message = serde_json::to_string(&self.encoded_call_message)?;
        let details = types::TxDetails {
            chain_id: self.details.chain_id,
            max_priority_fee_bips: self.details.max_priority_fee_bips.0,
            max_fee: types::TxDetailsMaxFee::try_from(self.details.max_fee.to_string())
                .expect("Failed to convert max fee from string"),
            gas_limit: self
                .details
                .gas_limit
                .map(|gas_limit| types::GasUnit(gas_limit.as_ref().to_vec())),
        };
        let generation = self.generation;
        let gas_price = self.gas_price.map(|price| {
            types::GasPrice(
                price
                    .as_ref()
                    .iter()
                    .map(|item| {
                        item.to_string()
                            .try_into()
                            .expect("Api spec rejected valid integer. This should never happen.")
                    })
                    .collect::<Vec<types::GasPriceItem>>(),
            )
        });

        let sequencer = self
            .sequencer
            .map(|sequencer| serde_json::to_string(&sequencer))
            .transpose()?;

        let sequencer_rollup_address = self
            .sequencer_rollup_address
            .map(|address| serde_json::to_string(&address))
            .transpose()?;

        Ok(types::PartialTransaction {
            sender_pub_key,
            generation,
            encoded_call_message,
            details,
            gas_price,
            sequencer,
            sequencer_rollup_address,
        })
    }
}

impl<S: Spec> TryFrom<types::PartialTransaction> for crate::PartialTransaction<S> {
    type Error = anyhow::Error;

    fn try_from(value: types::PartialTransaction) -> Result<Self, Self::Error> {
        let sender_pub_key = serde_json::from_str(&value.sender_pub_key)?;
        let encoded_call_message = serde_json::from_str(&value.encoded_call_message)?;
        let details = TxDetails::<S> {
            chain_id: value.details.chain_id,
            max_priority_fee_bips: PriorityFeeBips(value.details.max_priority_fee_bips),
            max_fee: value.details.max_fee.as_str().parse::<Amount>()?,
            gas_limit: value
                .details
                .gas_limit
                .map(|gas_limit| S::Gas::try_from(gas_limit.0).map_err(Into::into))
                .transpose()?,
        };
        let generation = value.generation;
        let gas_price = value
            .gas_price
            .map(|gas_price| {
                gas_price
                    .0
                    .iter()
                    .map(|item| item.as_str().parse::<Amount>())
                    .collect::<Result<Vec<_>, ParseIntError>>()
            })
            .transpose()?
            .map(|g| <S::Gas as Gas>::Price::try_from(g).map_err(Into::into))
            .transpose()?;

        let sequencer = value
            .sequencer
            .map(|sequencer| serde_json::from_str(&sequencer))
            .transpose()?;

        let sequencer_rollup_address = value
            .sequencer_rollup_address
            .map(|address| serde_json::from_str(&address))
            .transpose()?;

        Ok(Self {
            sender_pub_key,
            generation,
            encoded_call_message,
            details,
            gas_price,
            sequencer,
            sequencer_rollup_address,
        })
    }
}
