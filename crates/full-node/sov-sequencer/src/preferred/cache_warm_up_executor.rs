#![allow(dead_code)]
use crate::preferred::block_executor::StartBlockData;
use crate::preferred::next_visible_slot_number_increase;
use crate::preferred::PreferredSequencerConfig;
use crate::preferred::RollupBlockExecutor;
use crate::preferred::RollupBlockExecutorConfig;
use crate::SequencerConfig;
use sov_modules_api::Spec;
use sov_modules_api::StateUpdateInfo;
use sov_modules_api::VersionReader;
use sov_modules_api::{FullyBakedTx, Runtime};
use sov_state::NativeStorage;
use tokio::task::JoinHandle;

#[derive(Clone)]
pub(crate) struct CacheWarmupExecutor {
    start_batch_notification_sender: tokio::sync::broadcast::Sender<()>,
    tx_sender: flume::Sender<FullyBakedTx>,
}

impl CacheWarmupExecutor {
    pub(crate) fn send_batch_start_notification(&self) {
        //println!("Start send_batch_start_notification");
        let _ = self.start_batch_notification_sender.send(());
        //println!("End send_batch_start_notification");
    }

    pub(crate) async fn send_tx(&self, tx: FullyBakedTx) {
        //println!("Start send_tx");
        let _ = self.tx_sender.send_async(tx).await;
        //println!("End send_tx");
    }

    pub(crate) async fn spawn_execution_task<S: Spec, Rt: Runtime<S>>(
        info: StateUpdateInfo<S::Storage>,
        exec_config: RollupBlockExecutorConfig<S>,
        seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
    ) -> (Self, Vec<JoinHandle<()>>) {
        println!("spawn_execution_task");
        let (tx_sender, tx_receiver) = flume::bounded(100);
        let (start_batch_notification_sender, _) = tokio::sync::broadcast::channel::<()>(100);

        let mut handles = Vec::new();
        for _ in 0..1 {
            let worker = Self::spawn_worker::<S, Rt>(
                info.clone(),
                exec_config.clone(),
                seq_config.clone(),
                tx_receiver.clone(),
                start_batch_notification_sender.subscribe(),
            );

            handles.push(worker);
        }

        (
            Self {
                tx_sender,
                start_batch_notification_sender,
            },
            handles,
        )
    }

    fn spawn_worker<S: Spec, Rt: Runtime<S>>(
        info: StateUpdateInfo<S::Storage>,
        exec_config: RollupBlockExecutorConfig<S>,
        seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
        tx_receiver: flume::Receiver<FullyBakedTx>,
        mut start_batch_notification_sender: tokio::sync::broadcast::Receiver<()>,
    ) -> JoinHandle<()> {
        println!("spawn_worker");
        tokio::spawn(async move {
            let mut executor =
                RollupBlockExecutor::<_, Rt>::new(&info, exec_config, seq_config.clone());

            // Self::start_block(&seq_config, &info, &mut executor).await;

            loop {
                tokio::select! {
                    notify = start_batch_notification_sender.recv() => {
                        match notify{
                            Ok(_) => {


                                //executor.shutdown().await;
                                // TODO replace state
                                //Self::start_block(&seq_config, &info, &mut executor).await;
                                println!("recv start block");
                            },
                            Err(e) =>{ todo!() }
                        }
                    }

                    tx = tx_receiver.recv_async() => {
                        let baked_tx = match tx {
                             Ok(tx) => tx,
                             Err(e) => todo!(),
                        };
                        //let _ = executor.apply_tx_to_in_progress_batch(&baked_tx).await;
                        println!("recv tx");
                    }
                }
            }
        })
    }

    async fn start_block<S: Spec, Rt: Runtime<S>>(
        seq_config: &SequencerConfig<S::Address, PreferredSequencerConfig>,
        info: &StateUpdateInfo<S::Storage>,
        executor: &mut RollupBlockExecutor<S, Rt>,
    ) {
        let node_state_root = info.storage.get_root_hash(info.slot_number).unwrap();

        let visible_increase = next_visible_slot_number_increase(
            &executor.checkpoint,
            &info,
            false, // todo
            seq_config
                .sequencer_kind_config
                .ideal_lag_behind_finalized_slot,
        )
        .unwrap();

        let sanity_check_visible_slot_number_after_increase = executor
            .checkpoint
            .current_visible_slot_number()
            .advance(visible_increase.get().into());

        let min_profit_per_tx = seq_config.sequencer_kind_config.minimum_profit_per_tx;

        let start_block_data = StartBlockData {
            sanity_check_visible_slot_number_after_increase,
            visible_increase,
            node_state_root: node_state_root.clone(),
        };

        executor
            .start_rollup_block(start_block_data, min_profit_per_tx)
            .await;
    }
}
