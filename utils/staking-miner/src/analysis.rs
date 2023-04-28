// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

//! The analysis command.

use crate::{
	dry_run::force_create_snapshot, opts::AnalysisConfig, prelude::*, Error, SharedRpcClient,
};
use codec::Encode;
use frame_support::traits::Currency;
use remote_externalities::{Builder, Mode, OfflineConfig, OnlineConfig, SnapshotConfig, Transport};
use sp_npos_elections::ElectionScore;
use sp_runtime::DeserializeOwned;

#[derive(Debug, Clone, clap::Parser)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) enum AnalysisCommand {
	Extract,
	TransformElection,
}

/// Helper method to print the encoded size of the snapshot.
pub async fn print_info<T: EPM::Config>(
	ext: &mut Ext,
	raw_solution: &EPM::RawSolution<EPM::SolutionOf<T::MinerConfig>>,
) where
	<T as EPM::Config>::Currency: Currency<T::AccountId, Balance = Balance>,
{
	ext.execute_with(|| {
		log::info!(
			target: LOG_TARGET,
			"Snapshot Metadata: {:?}",
			<EPM::Pallet<T>>::snapshot_metadata()
		);
		log::info!(
			target: LOG_TARGET,
			"Snapshot Encoded Length: {:?}",
			<EPM::Pallet<T>>::snapshot()
				.expect("snapshot must exist before calling `measure_snapshot_size`")
				.encode()
				.len()
		);

		let snapshot_size =
			<EPM::Pallet<T>>::snapshot_metadata().expect("snapshot must exist by now; qed.");
		let deposit = EPM::Pallet::<T>::deposit_for(raw_solution, snapshot_size);

		let score = {
			let ElectionScore { minimal_stake, sum_stake, sum_stake_squared } = raw_solution.score;
			[Token::from(minimal_stake), Token::from(sum_stake), Token::from(sum_stake_squared)]
		};

		log::info!(
			target: LOG_TARGET,
			"solution score {:?} / deposit {:?} / length {:?}",
			score,
			Token::from(deposit),
			raw_solution.encode().len(),
		);
	});
}

pub(crate) async fn create_local_ext<B>(path: &str) -> Result<Ext, &'static str>
where
	B: BlockT + DeserializeOwned,
	B::Header: DeserializeOwned,
{
	Builder::<B>::new()
		.mode(Mode::Offline(OfflineConfig { state_snapshot: SnapshotConfig::new(path) }))
		.build()
		.await
		.map_err(|why| why)
		.map(|rx| rx.inner_ext)
}

// this indirection can probably be removed to simplify.
macro_rules! analysis_cmd_for {
	($runtime:ident) => {
		paste::paste! {
			/// Execute the analysis command.
			pub(crate) async fn [<analysis_cmd_ $runtime>](
				rpc: SharedRpcClient,
				config: AnalysisConfig,
			) -> Result<(), Error<$crate::[<$runtime _runtime_exports>]::Runtime>> {

				// eventually needs to be used. we'll need the Staking, EPM and BagsList pallet keys for
				// this data analysis.
				let _pallets = if config.force_snapshot {
					vec!["Staking".to_string(), "BagsList".to_string()]
				} else {
					// for now, let's stick with EPM only
					Default::default()
				};

				// this is a draft to be used when fetching data from local snapshots. We'll need somehthing
				// like this when loading the data from snapshots and perform the data analysis.
				//let ext  = create_local_ext::<Block>(&config.path).await.unwrap(); // todo handle error

				match config.command {
					AnalysisCommand::Extract => [<extract_for_ $runtime>](rpc, config).await,
					AnalysisCommand::TransformElection => [<election_operation_for_ $runtime>](config).await,
				}
			}
		}
	};
}

macro_rules! extract_for {
	($runtime:ident) => {
		paste::paste! {
			pub(crate) async fn [<extract_for_ $runtime>](
				rpc: SharedRpcClient,
				config: AnalysisConfig,
			)  -> Result<(), Error<$crate::[<$runtime _runtime_exports>]::Runtime>> {
				use $crate::[<$runtime _runtime_exports>]::*;

				use frame_support::{storage::generator::StorageMap, traits::PalletInfo};
				use sp_core::hashing::twox_128;

                let file_path = format!("{}/{}.data", config.path, config.at.unwrap()); // handle unwrap

				let additional_pallets = vec!["Staking".to_string(), "BagsList".to_string()]; // take this from configs.

                let mut pallets = vec![<Runtime as frame_system::Config>::PalletInfo::name::<EPM::Pallet<Runtime>>()
		            .expect("Pallet always has name; qed.")
		            .to_string()];
                pallets.extend(additional_pallets);

				log::info!(target: LOG_TARGET, "Scrapping keys for pallets {:?} in block {:?}", pallets, config.at.unwrap()); // handle unwrap

				Builder::<Block>::new()
					.mode(Mode::Online(OnlineConfig {
						transport: Transport::Uri(rpc.uri().to_owned()),
						at: config.at,
						pallets,
						hashed_prefixes: vec![<frame_system::BlockHash<Runtime>>::prefix_hash()],
						hashed_keys: vec![[twox_128(b"System"), twox_128(b"Number")].concat()],
						state_snapshot: Some(file_path.clone().into()),
						..Default::default()
					}))
					.build()
                    .await.unwrap(); // handle unwrap

				log::info!(target: LOG_TARGET, "\nDone, check {:?}", file_path);

				Ok(())
			}
		}
	};
}

macro_rules! election_operation_for {
	($runtime:ident) => {
		paste::paste! {
			pub(crate) async fn [<election_operation_for_ $runtime>](
				config: AnalysisConfig,
			)  -> Result<(), Error<$crate::[<$runtime _runtime_exports>]::Runtime>> {
				use $crate::[<$runtime _runtime_exports>]::*;

				//if config.force_snapshot {
				//	force_create_snapshot::<Runtime>(&mut ext)?;
				//};

                let file_path = format!("{}/{}.data", config.path, config.at.unwrap()); // handle unwrap

                let mut ext = Builder::<Block>::new()
                    .mode(Mode::Offline(OfflineConfig {
				        state_snapshot: SnapshotConfig::new(file_path.clone()),
                }))
                .build()
                .await.unwrap(); // handle unwrap

                log::info!(target: LOG_TARGET, "Loaded Ext from file {:?}", file_path);

				let solver = crate::opts::Solver::SeqPhragmen{iterations: 10};   // todo: from config

				let raw_solution = crate::mine_with::<Runtime>(&solver, &mut ext, false)?;
				print_info::<Runtime>(&mut ext, &raw_solution).await;
				let feasibility_result = ext.execute_with(|| {
					EPM::Pallet::<Runtime>::feasibility_check(raw_solution.clone(), EPM::ElectionCompute::Signed)
				});
				log::info!(target: LOG_TARGET, "feasibility result is {:?}", feasibility_result.map(|_| ()));

				Ok(())
			}
		}
	};
}

analysis_cmd_for!(polkadot);
analysis_cmd_for!(kusama);
analysis_cmd_for!(westend);

extract_for!(polkadot);
extract_for!(kusama);
extract_for!(westend);

election_operation_for!(polkadot);
election_operation_for!(kusama);
election_operation_for!(westend);
