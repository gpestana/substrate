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

use std::marker::PhantomData;

use crate::{
	opts::{AnalysisConfig, Solver},
	prelude::*,
	Error, SharedRpcClient,
};
use codec::Encode;
use frame_election_provider_support::SortedListProvider;
use frame_system::pallet_prelude::BlockNumberFor;
use remote_externalities::{Builder, Mode, OfflineConfig, OnlineConfig, SnapshotConfig, Transport};
use serde::{Deserialize, Serialize};
use sp_npos_elections::ElectionScore;
use EPM::{BalanceOf, SolutionOrSnapshotSize};

const NPOS_MAX_ITERATIONS_COEFFICIENT: u32 = 2;

#[derive(Debug, Clone, clap::Parser)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) enum AnalysisCommand {
	Extract,
	TransformElection,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ElectionEntryCSV<T: EPM::Config> {
	block_number: u32,
	phrag_min_stake: u128,
	phrag_sum_stake: u128,
	phrag_sum_stake_squared: u128,
	mms_min_stake: u128,
	mms_sum_stake: u128,
	mms_sum_stake_squared: u128,
	dpos_min_stake: u128,
	dpos_sum_stake: u128,
	dpos_sum_stake_squared: u128,
	dpos_unbound_min_stake: u128,
	dpos_unbound_sum_stake: u128,
	dpos_unbound_sum_stake_squared: u128,
	voters: u32,
	targets: u32,
	snapshot_size: usize,
	min_active_stake: u128,
	#[serde(skip)]
	_marker: PhantomData<T>,
}

impl<T: EPM::Config> ElectionEntryCSV<T> {
	fn new(
		block_number: BlockNumberFor<T>,
		phrag_solutions: (
			&EPM::RawSolution<EPM::SolutionOf<T::MinerConfig>>,
			&EPM::RawSolution<EPM::SolutionOf<T::MinerConfig>>,
		),
		dpos_score: (u128, u128, u128),
		dpos_unbounded_score: (u128, u128, u128),
		snapshot_metadata: SolutionOrSnapshotSize,
		snapshot_size: usize,
		min_active_stake: BalanceOf<T>,
	) -> Self
	where
		BlockNumberFor<T>: Into<u32>,
		BalanceOf<T>: Into<u128>,
	{
		let (phrag_min_stake, phrag_sum_stake, phrag_sum_stake_squared) = {
			let ElectionScore { minimal_stake, sum_stake, sum_stake_squared } =
				phrag_solutions.0.score;
			(minimal_stake, sum_stake, sum_stake_squared)
		};

		let (mms_min_stake, mms_sum_stake, mms_sum_stake_squared) = {
			let ElectionScore { minimal_stake, sum_stake, sum_stake_squared } =
				phrag_solutions.1.score;
			(minimal_stake, sum_stake, sum_stake_squared)
		};

		let SolutionOrSnapshotSize { voters, targets } = snapshot_metadata;

		Self {
			block_number: block_number.into(),
			phrag_min_stake,
			phrag_sum_stake,
			phrag_sum_stake_squared,
			mms_min_stake,
			mms_sum_stake,
			mms_sum_stake_squared,
			dpos_min_stake: dpos_score.0,
			dpos_sum_stake: dpos_score.1,
			dpos_sum_stake_squared: dpos_score.2,
			dpos_unbound_min_stake: dpos_unbounded_score.0,
			dpos_unbound_sum_stake: dpos_unbounded_score.1,
			dpos_unbound_sum_stake_squared: dpos_unbounded_score.2,
			voters,
			targets,
			snapshot_size,
			min_active_stake: min_active_stake.into(),
			_marker: PhantomData,
		}
	}
}

pub(crate) fn snapshot_data<T: EPM::Config>(ext: &mut Ext) -> (SolutionOrSnapshotSize, usize) {
	ext.execute_with(|| {
		if <EPM::Snapshot<T>>::exists() {
			log::info!(target: LOG_TARGET, "snapshot already exists.");
		} else {
			log::info!(target: LOG_TARGET, "creating a fake snapshot now.");
			<EPM::Pallet<T>>::create_snapshot().unwrap();
		};

		(
			<EPM::SnapshotMetadata<T>>::get().expect("snapshot metadata should exist by now. qed."),
			<EPM::Pallet<T>>::snapshot()
				.expect("snapshot should exist by now. qed.")
				.encode()
				.len(),
		)
	})
}

pub(crate) fn min_active_stake<T: EPM::Config + Staking::Config>(ext: &mut Ext) -> BalanceOf<T>
where
	BalanceOf<T>: From<u64>,
{
	ext.execute_with(|| {
		let weight_of = pallet_staking::Pallet::<T>::weight_of_fn();

		let maybe_max_len = None; // get this from somewhere.

		let max_allowed_len = {
			let all_voter_count = T::VoterList::count() as usize;
			maybe_max_len.unwrap_or(all_voter_count).min(all_voter_count)
		};

		let mut all_voters = Vec::<_>::with_capacity(max_allowed_len);
		let mut min_active_stake = u64::MAX;
		let mut voters_seen = 0u32;

		let mut sorted_voters = T::VoterList::iter();
		while all_voters.len() < max_allowed_len &&
			voters_seen < (NPOS_MAX_ITERATIONS_COEFFICIENT * max_allowed_len as u32)
		{
			let voter = match sorted_voters.next() {
				Some(voter) => {
					voters_seen += 1;
					voter
				},
				None => break,
			};

			let voter_weight = weight_of(&voter);

			min_active_stake =
				if voter_weight < min_active_stake { voter_weight } else { min_active_stake };

			// it doesn't really matter here.
			all_voters.push(min_active_stake);
		}

		min_active_stake.into()
	})
}

pub(crate) fn block_number<T: EPM::Config>(ext: &mut Ext) -> BlockNumberFor<T> {
	ext.execute_with(|| <frame_system::Pallet<T>>::block_number())
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

                let file_path = format!("{}/{}.data", config.path, config.at.unwrap()); // handle unwrap

                let mut ext = Builder::<Block>::new()
                    .mode(Mode::Offline(OfflineConfig {
				        state_snapshot: SnapshotConfig::new(file_path.clone()),
                }))
                .build()
                .await.expect("snapshot should decode as expected.");

                log::info!(target: LOG_TARGET, "Loaded Ext from file {:?}", file_path);

                // it also forces creation of a new snapshot, if it does not exist.
                let (snapshot_metadata, snapshot_size) = snapshot_data::<Runtime>(&mut ext);

                let min_active_stake = min_active_stake::<Runtime>(&mut ext);
                let block_number = block_number::<Runtime>(&mut ext);

                let phrag_raw_solution = crate::mine_with::<Runtime>(&Solver::SeqPhragmen{iterations: 10}, &mut ext, false).unwrap();
                let mms_raw_solution = crate::mine_with::<Runtime>(&Solver::PhragMMS{iterations: 10}, &mut ext, false).unwrap();

                let dpos_score = crate::mine_dpos::<Runtime>(&mut ext, true).unwrap();
                let dpos_unbound_score = crate::mine_dpos::<Runtime>(&mut ext, false).unwrap();

                // csv
                let entry = ElectionEntryCSV::<Runtime>::new(
                    block_number,
                    (&phrag_raw_solution, &mms_raw_solution),
                    dpos_score,
                    dpos_unbound_score,
                    snapshot_metadata,
                    snapshot_size,
                    min_active_stake,
                );

                log::info!(target: LOG_TARGET, "\n{:#?}", entry);

                let headers = if std::path::Path::new(&config.csv).exists() { false }  else { true };
                let csv = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .append(true)
                    .open(&config.csv)
                    .unwrap();

                let mut buffer = csv::WriterBuilder::new().has_headers(headers).from_writer(csv);
                buffer.serialize(entry).unwrap();
                buffer.flush().unwrap();

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
