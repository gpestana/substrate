use crate::tests::{AccountId, BlockNumber, MaxVotesPerVoter};

use frame_election_provider_support::ElectionDataProvider;

pub struct DataProvider;
impl ElectionDataProvider for DataProvider {
	type AccountId = AccountId;
	type BlockNumber = BlockNumber;
	type MaxVotesPerVoter = MaxVotesPerVoter;

	fn electable_targets(
		maybe_max_len: Option<usize>,
	) -> frame_election_provider_support::data_provider::Result<Vec<Self::AccountId>> {
		//TODO(gpestana): implement
		Ok(vec![])
	}

	fn electing_voters(
		maybe_max_len: Option<usize>,
	) -> frame_election_provider_support::data_provider::Result<
		Vec<frame_election_provider_support::VoterOf<Self>>,
	> {
		Ok(vec![])
	}

	fn desired_targets() -> frame_election_provider_support::data_provider::Result<u32> {
		Ok(10)
	}

	fn next_election_prediction(now: Self::BlockNumber) -> Self::BlockNumber {
		0
	}
}
