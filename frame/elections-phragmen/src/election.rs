use sp_std::{collections::btree_map::BTreeMap, marker::PhantomData, prelude::*};

use crate::{BalanceOf, Voter};
use frame_election_provider_support::{
	weights::WeightInfo, ElectionDataProvider, ElectionProvider, ElectionProviderBase, NposSolver,
};
use frame_support::{dispatch::DispatchClass, traits::Get};
use sp_npos_elections::*;

type AccountId<T> = <T as frame_system::Config>::AccountId;
type BlockNumber<T> = <T as frame_system::Config>::BlockNumber;

/// Errors of the on-chain election.
#[derive(Eq, PartialEq, Debug)]
pub enum Error {
	/// An internal error in the NPoS elections crate.
	NposElections(sp_npos_elections::Error),
	/// Errors from the data provider.
	DataProvider(&'static str),
}

impl From<sp_npos_elections::Error> for Error {
	fn from(e: sp_npos_elections::Error) -> Self {
		Error::NposElections(e)
	}
}

pub trait DataProviderConfig {
	type System: frame_system::Config;
	type MaxVotesPerVoter: Get<u32>;

	fn candidates() -> Vec<AccountId<Self::System>>;

	// TODO(gpestana): change trait bound to BalancesOf
	fn votes_with_stake(
	) -> Vec<(AccountId<Self::System>, Voter<AccountId<Self::System>, AccountId<Self::System>>)>;
}

pub struct DataProvider<T: DataProviderConfig>(PhantomData<T>);
impl<T: DataProviderConfig> ElectionDataProvider for DataProvider<T> {
	type AccountId = AccountId<T::System>;
	type BlockNumber = BlockNumber<T::System>;
	type MaxVotesPerVoter = T::MaxVotesPerVoter;

	fn electable_targets(
		maybe_max_len: Option<usize>,
	) -> frame_election_provider_support::data_provider::Result<Vec<Self::AccountId>> {
		Ok(T::candidates())
	}

	fn electing_voters(
		maybe_max_len: Option<usize>,
	) -> frame_election_provider_support::data_provider::Result<
		Vec<frame_election_provider_support::VoterOf<Self>>,
	> {
		// in frame_elections_support:
		// VoterOf = (AccountId, VoteWeight, BoundedVec<AccountId, Bound>), where Bound = MaxVotesPerVoter
		// Vec<VoterOf<AccountId, MaVotesPerVoter>>

		// A voter, at the level of abstraction of this crate.
		// pub type Voter<AccountId, Bound> = (AccountId, VoteWeight, BoundedVec<AccountId, Bound>); where
		// - AccountId: voter ID
		// - VoteWeight: voter's stake
		// - BoundedVec<AccountId, Bound> is a bounded vec with the ids of the validators to vote

		//let votes = Vec::new();

		println!("Votes with stake: {:?}", T::votes_with_stake());

		Ok(vec![])
	}

	fn desired_targets() -> frame_election_provider_support::data_provider::Result<u32> {
		Ok(1)
	}

	fn next_election_prediction(now: Self::BlockNumber) -> Self::BlockNumber {
		<frame_system::Pallet<T::System>>::block_number()
	}
}

/*
// use frame_support_elections::BoundedExecution instead
pub trait ElectionConfig {
	type System: frame_system::Config;
	type DataProvider: ElectionDataProvider<
		AccountId = <Self::System as frame_system::Config>::AccountId,
		BlockNumber = <Self::System as frame_system::Config>::BlockNumber,
	>;
	type Solver: NposSolver<
		AccountId = <Self::System as frame_system::Config>::AccountId,
		Error = sp_npos_elections::Error,
	>;
	type WeightInfo: WeightInfo;
}

pub struct BoundedExecution<T: ElectionConfig>(PhantomData<T>);

impl<T: ElectionConfig> ElectionProviderBase for BoundedExecution<T> {
	type AccountId = <T::System as frame_system::Config>::AccountId;
	type BlockNumber = <T::System as frame_system::Config>::BlockNumber;
	type Error = Error;
	type DataProvider = T::DataProvider;

	fn ongoing() -> bool {
		return false;
	}
}

impl<T: ElectionConfig> ElectionProvider for BoundedExecution<T> {
	fn elect() -> Result<frame_election_provider_support::Supports<Self::AccountId>, Self::Error> {
		Err(Error::DataProvider("noop"))
	}
}
*/
