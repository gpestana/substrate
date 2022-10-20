use crate::tests::*;

use frame_election_provider_support::ElectionDataProvider;

pub struct DataProvider;
impl ElectionDataProvider for DataProvider {
	type AccountId = AccountId;
	type BlockNumber = BlockNumber;
	type MaxVotesPerVoter = MaxVotesPerVoter;

	fn electable_targets(
		maybe_max_len: Option<usize>,
	) -> frame_election_provider_support::data_provider::Result<Vec<Self::AccountId>> {
		let (mut targets, _): (Vec<_>, Vec<_>) = Elections::candidates().into_iter().unzip();

		if let Some(max_len) = maybe_max_len {
			targets.truncate(max_len)
		}

		Ok(targets)
	}

	fn electing_voters(
		maybe_max_len: Option<usize>,
	) -> frame_election_provider_support::data_provider::Result<
		Vec<frame_election_provider_support::VoterOf<Self>>,
	> {
		//TODO(gpestana)
		Ok(vec![])
	}

	fn desired_targets() -> frame_election_provider_support::data_provider::Result<u32> {
		Ok(DesiredMembers::get())
	}

	fn next_election_prediction(now: Self::BlockNumber) -> Self::BlockNumber {
		// TODO(gpestana)
		0
	}
}
