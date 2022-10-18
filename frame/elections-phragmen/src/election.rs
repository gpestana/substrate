use core::marker::PhantomData;

use frame_election_provider_support::{
	ElectionDataProvider, ElectionProvider, ElectionProviderBase,
};

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

pub trait Config {
	type System: frame_system::Config;
	type DataProvider: ElectionDataProvider<
		AccountId = <Self::System as frame_system::Config>::AccountId,
		BlockNumber = <Self::System as frame_system::Config>::BlockNumber,
	>;
}

pub struct BoundedExecution<T: Config>(PhantomData<T>);
pub struct BoundedExecutionDataProvider<T: Config>(PhantomData<T>);

impl<T: Config> ElectionProviderBase for BoundedExecution<T> {
	type AccountId = <T::System as frame_system::Config>::AccountId;
	type BlockNumber = <T::System as frame_system::Config>::BlockNumber;
	type Error = Error;
	type DataProvider = T::DataProvider;

	fn ongoing() -> bool {
		return false; // TODO(gpestana): where to fetch this from?
	}
}

impl<T: Config> ElectionProvider for BoundedExecution<T> {
	fn elect() -> Result<frame_election_provider_support::Supports<Self::AccountId>, Self::Error> {
		// TODO(gpestana): implement

		println!("election::BoundedExecution::elect()");

		Err(Error::DataProvider("noop mock"))
	}
}
