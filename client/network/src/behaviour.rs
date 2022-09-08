// This file is part of Substrate.

// Copyright (C) 2019-2022 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use crate::{
	bitswap::Bitswap,
	discovery::{DiscoveryBehaviour, DiscoveryConfig, DiscoveryOut},
	peer_info,
	protocol::{message::Roles, CustomMessageOutcome, NotificationsSink, Protocol},
	request_responses,
};

use bytes::Bytes;
use futures::channel::oneshot;
use libp2p::{
	core::{Multiaddr, PeerId, PublicKey},
	identify::IdentifyInfo,
	kad::record,
	swarm::{behaviour::toggle::Toggle, NetworkBehaviour, NetworkBehaviourAction, PollParameters},
	NetworkBehaviour,
};
use log::debug;

use sc_consensus::import_queue::{IncomingBlock, Origin};
use sc_network_common::{
	config::ProtocolId,
	protocol::{
		event::{DhtEvent, ObservedRole},
		ProtocolName,
	},
	request_responses::{IfDisconnected, ProtocolConfig, RequestFailure},
	sync::{warp::WarpProofRequest, OpaqueBlockRequest, OpaqueStateRequest},
};
use sc_peerset::{PeersetHandle, ReputationChange};
use sp_blockchain::HeaderBackend;
use sp_consensus::BlockOrigin;
use sp_runtime::{
	traits::{Block as BlockT, NumberFor},
	Justifications,
};
use std::{
	collections::{HashSet, VecDeque},
	iter,
	task::{Context, Poll},
	time::Duration,
};

pub use crate::request_responses::{InboundFailure, OutboundFailure, RequestId, ResponseFailure};

/// General behaviour of the network. Combines all protocols together.
#[derive(NetworkBehaviour)]
#[behaviour(out_event = "BehaviourOut<B>", poll_method = "poll", event_process = true)]
pub struct Behaviour<B, Client>
where
	B: BlockT,
	Client: HeaderBackend<B> + 'static,
{
	/// All the substrate-specific protocols.
	substrate: Protocol<B, Client>,
	/// Periodically pings and identifies the nodes we are connected to, and store information in a
	/// cache.
	peer_info: peer_info::PeerInfoBehaviour,
	/// Discovers nodes of the network.
	discovery: DiscoveryBehaviour,
	/// Bitswap server for blockchain data.
	bitswap: Toggle<Bitswap<B>>,
	/// Generic request-response protocols.
	request_responses: request_responses::RequestResponsesBehaviour,

	/// Queue of events to produce for the outside.
	#[behaviour(ignore)]
	events: VecDeque<BehaviourOut<B>>,
}

/// Event generated by `Behaviour`.
pub enum BehaviourOut<B: BlockT> {
	BlockImport(BlockOrigin, Vec<IncomingBlock<B>>),
	JustificationImport(Origin, B::Hash, NumberFor<B>, Justifications),

	/// Started a random iterative Kademlia discovery query.
	RandomKademliaStarted(ProtocolId),

	/// We have received a request from a peer and answered it.
	///
	/// This event is generated for statistics purposes.
	InboundRequest {
		/// Peer which sent us a request.
		peer: PeerId,
		/// Protocol name of the request.
		protocol: ProtocolName,
		/// If `Ok`, contains the time elapsed between when we received the request and when we
		/// sent back the response. If `Err`, the error that happened.
		result: Result<Duration, ResponseFailure>,
	},

	/// A request has succeeded or failed.
	///
	/// This event is generated for statistics purposes.
	RequestFinished {
		/// Peer that we send a request to.
		peer: PeerId,
		/// Name of the protocol in question.
		protocol: ProtocolName,
		/// Duration the request took.
		duration: Duration,
		/// Result of the request.
		result: Result<(), RequestFailure>,
	},

	/// A request protocol handler issued reputation changes for the given peer.
	ReputationChanges {
		peer: PeerId,
		changes: Vec<ReputationChange>,
	},

	/// Opened a substream with the given node with the given notifications protocol.
	///
	/// The protocol is always one of the notification protocols that have been registered.
	NotificationStreamOpened {
		/// Node we opened the substream with.
		remote: PeerId,
		/// The concerned protocol. Each protocol uses a different substream.
		protocol: ProtocolName,
		/// If the negotiation didn't use the main name of the protocol (the one in
		/// `notifications_protocol`), then this field contains which name has actually been
		/// used.
		/// See also [`crate::Event::NotificationStreamOpened`].
		negotiated_fallback: Option<ProtocolName>,
		/// Object that permits sending notifications to the peer.
		notifications_sink: NotificationsSink,
		/// Role of the remote.
		role: ObservedRole,
	},

	/// The [`NotificationsSink`] object used to send notifications with the given peer must be
	/// replaced with a new one.
	///
	/// This event is typically emitted when a transport-level connection is closed and we fall
	/// back to a secondary connection.
	NotificationStreamReplaced {
		/// Id of the peer we are connected to.
		remote: PeerId,
		/// The concerned protocol. Each protocol uses a different substream.
		protocol: ProtocolName,
		/// Replacement for the previous [`NotificationsSink`].
		notifications_sink: NotificationsSink,
	},

	/// Closed a substream with the given node. Always matches a corresponding previous
	/// `NotificationStreamOpened` message.
	NotificationStreamClosed {
		/// Node we closed the substream with.
		remote: PeerId,
		/// The concerned protocol. Each protocol uses a different substream.
		protocol: ProtocolName,
	},

	/// Received one or more messages from the given node using the given protocol.
	NotificationsReceived {
		/// Node we received the message from.
		remote: PeerId,
		/// Concerned protocol and associated message.
		messages: Vec<(ProtocolName, Bytes)>,
	},

	/// A new block request must be emitted.
	BlockRequest {
		/// Node we send the request to.
		target: PeerId,
		/// Opaque implementation-specific block request.
		request: OpaqueBlockRequest,
		/// One-shot channel to recieve the response.
		pending_response: oneshot::Sender<Result<Vec<u8>, RequestFailure>>,
	},

	/// A new state request must be emitted.
	StateRequest {
		/// Node we send the request to.
		target: PeerId,
		/// Opaque implementation-specific state request.
		request: OpaqueStateRequest,
		/// One-shot channel to recieve the response.
		pending_response: oneshot::Sender<Result<Vec<u8>, RequestFailure>>,
	},

	/// A new warp sync request must be emitted.
	WarpSyncRequest {
		/// Node we send the request to.
		target: PeerId,
		/// Warp sync request.
		request: WarpProofRequest<B>,
		/// One-shot channel to recieve the response.
		pending_response: oneshot::Sender<Result<Vec<u8>, RequestFailure>>,
	},

	/// Now connected to a new peer for syncing purposes.
	SyncConnected(PeerId),

	/// No longer connected to a peer for syncing purposes.
	SyncDisconnected(PeerId),

	/// We have obtained identity information from a peer, including the addresses it is listening
	/// on.
	PeerIdentify {
		/// Id of the peer that has been identified.
		peer_id: PeerId,
		/// Information about the peer.
		info: IdentifyInfo,
	},

	/// Events generated by a DHT as a response to get_value or put_value requests as well as the
	/// request duration.
	Dht(DhtEvent, Duration),

	/// Ignored event generated by lower layers.
	None,
}

impl<B, Client> Behaviour<B, Client>
where
	B: BlockT,
	Client: HeaderBackend<B> + 'static,
{
	/// Builds a new `Behaviour`.
	pub fn new(
		substrate: Protocol<B, Client>,
		user_agent: String,
		local_public_key: PublicKey,
		disco_config: DiscoveryConfig,
		block_request_protocol_config: ProtocolConfig,
		state_request_protocol_config: ProtocolConfig,
		warp_sync_protocol_config: Option<ProtocolConfig>,
		bitswap: Option<Bitswap<B>>,
		light_client_request_protocol_config: ProtocolConfig,
		// All remaining request protocol configs.
		mut request_response_protocols: Vec<ProtocolConfig>,
		peerset: PeersetHandle,
	) -> Result<Self, request_responses::RegisterError> {
		if let Some(config) = warp_sync_protocol_config {
			request_response_protocols.push(config);
		}
		request_response_protocols.push(block_request_protocol_config);
		request_response_protocols.push(state_request_protocol_config);
		request_response_protocols.push(light_client_request_protocol_config);

		Ok(Self {
			substrate,
			peer_info: peer_info::PeerInfoBehaviour::new(user_agent, local_public_key),
			discovery: disco_config.finish(),
			bitswap: bitswap.into(),
			request_responses: request_responses::RequestResponsesBehaviour::new(
				request_response_protocols.into_iter(),
				peerset,
			)?,
			events: VecDeque::new(),
		})
	}

	/// Returns the list of nodes that we know exist in the network.
	pub fn known_peers(&mut self) -> HashSet<PeerId> {
		self.discovery.known_peers()
	}

	/// Adds a hard-coded address for the given peer, that never expires.
	pub fn add_known_address(&mut self, peer_id: PeerId, addr: Multiaddr) {
		self.discovery.add_known_address(peer_id, addr)
	}

	/// Returns the number of nodes in each Kademlia kbucket for each Kademlia instance.
	///
	/// Identifies Kademlia instances by their [`ProtocolId`] and kbuckets by the base 2 logarithm
	/// of their lower bound.
	pub fn num_entries_per_kbucket(
		&mut self,
	) -> impl ExactSizeIterator<Item = (&ProtocolId, Vec<(u32, usize)>)> {
		self.discovery.num_entries_per_kbucket()
	}

	/// Returns the number of records in the Kademlia record stores.
	pub fn num_kademlia_records(&mut self) -> impl ExactSizeIterator<Item = (&ProtocolId, usize)> {
		self.discovery.num_kademlia_records()
	}

	/// Returns the total size in bytes of all the records in the Kademlia record stores.
	pub fn kademlia_records_total_size(
		&mut self,
	) -> impl ExactSizeIterator<Item = (&ProtocolId, usize)> {
		self.discovery.kademlia_records_total_size()
	}

	/// Borrows `self` and returns a struct giving access to the information about a node.
	///
	/// Returns `None` if we don't know anything about this node. Always returns `Some` for nodes
	/// we're connected to, meaning that if `None` is returned then we're not connected to that
	/// node.
	pub fn node(&self, peer_id: &PeerId) -> Option<peer_info::Node> {
		self.peer_info.node(peer_id)
	}

	/// Initiates sending a request.
	pub fn send_request(
		&mut self,
		target: &PeerId,
		protocol: &str,
		request: Vec<u8>,
		pending_response: oneshot::Sender<Result<Vec<u8>, RequestFailure>>,
		connect: IfDisconnected,
	) {
		self.request_responses
			.send_request(target, protocol, request, pending_response, connect)
	}

	/// Returns a shared reference to the user protocol.
	pub fn user_protocol(&self) -> &Protocol<B, Client> {
		&self.substrate
	}

	/// Returns a mutable reference to the user protocol.
	pub fn user_protocol_mut(&mut self) -> &mut Protocol<B, Client> {
		&mut self.substrate
	}

	/// Add a self-reported address of a remote peer to the k-buckets of the supported
	/// DHTs (`supported_protocols`).
	pub fn add_self_reported_address(
		&self,
		peer_id: &PeerId,
		supported_protocols: impl Iterator<Item = impl AsRef<[u8]>>,
		addr: Multiaddr,
	) {
		self.discovery.add_self_reported_address(peer_id, supported_protocols, addr);
	}

	/// Start querying a record from the DHT. Will later produce either a `ValueFound` or a
	/// `ValueNotFound` event.
	pub fn get_value(&mut self, key: record::Key) {
		self.discovery.get_value(key);
	}

	/// Starts putting a record into DHT. Will later produce either a `ValuePut` or a
	/// `ValuePutFailed` event.
	pub fn put_value(&mut self, key: record::Key, value: Vec<u8>) {
		self.discovery.put_value(key, value);
	}
}

fn reported_roles_to_observed_role(roles: Roles) -> ObservedRole {
	if roles.is_authority() {
		ObservedRole::Authority
	} else if roles.is_full() {
		ObservedRole::Full
	} else {
		ObservedRole::Light
	}
}

impl<B> From<void::Void> for BehaviourOut<B> {
	fn from(event: void::Void) -> Self {
		void::unreachable(event)
	}
}

impl<B> From<CustomMessageOutcome<B>> for BehaviourOut<B> {
	fn from(event: CustomMessageOutcome<B>) -> Self {
		match event {
			CustomMessageOutcome::BlockImport(origin, blocks) =>
				BehaviourOut::BlockImport(origin, blocks),
			CustomMessageOutcome::JustificationImport(origin, hash, nb, justification) =>
				BehaviourOut::JustificationImport(origin, hash, nb, justification),
			CustomMessageOutcome::BlockRequest { target, request, pending_response } =>
				BehaviourOut::BlockRequest { target, request, pending_response },
			CustomMessageOutcome::StateRequest { target, request, pending_response } =>
				BehaviourOut::StateRequest { target, request, pending_response },
			CustomMessageOutcome::WarpSyncRequest { target, request, pending_response } =>
				BehaviourOut::WarpSyncRequest { target, request, pending_response },
			CustomMessageOutcome::NotificationStreamOpened {
				remote,
				protocol,
				negotiated_fallback,
				roles,
				notifications_sink,
			} => BehaviourOut::NotificationStreamOpened {
				remote,
				protocol,
				negotiated_fallback,
				role: reported_roles_to_observed_role(roles),
				notifications_sink,
			},
			CustomMessageOutcome::NotificationStreamReplaced {
				remote,
				protocol,
				notifications_sink,
			} => BehaviourOut::NotificationStreamReplaced { remote, protocol, notifications_sink },
			CustomMessageOutcome::NotificationStreamClosed { remote, protocol } =>
				BehaviourOut::NotificationStreamClosed { remote, protocol },
			CustomMessageOutcome::NotificationsReceived { remote, messages } =>
				BehaviourOut::NotificationsReceived { remote, messages },
			CustomMessageOutcome::PeerNewBest(_peer_id, _number) => BehaviourOut::None,
			CustomMessageOutcome::SyncConnected(peer_id) => BehaviourOut::SyncConnected(peer_id),
			CustomMessageOutcome::SyncDisconnected(peer_id) =>
				BehaviourOut::SyncDisconnected(peer_id),
			CustomMessageOutcome::None => BehaviourOut::None,
		}
	}
}

impl<B> From<request_responses::Event> for BehaviourOut<B> {
	fn from(event: request_responses::Event) -> Self {
		match event {
			request_responses::Event::InboundRequest { peer, protocol, result } =>
				BehaviourOut::InboundRequest { peer, protocol, result },
			request_responses::Event::RequestFinished { peer, protocol, duration, result } =>
				BehaviourOut::RequestFinished { peer, protocol, duration, result },
			request_responses::Event::ReputationChanges { peer, changes } =>
				BehaviourOut::ReputationChanges { peer, changes },
		}
	}
}

impl<B> From<peer_info::PeerInfoEvent> for BehaviourOut<B> {
	fn from(event: peer_info::PeerInfoEvent) -> Self {
		let peer_info::PeerInfoEvent::Identified { peer_id, info } = event;
		BehaviourOut::PeerIdentify { peer_id, info }
	}
}

impl<B, Client> NetworkBehaviourEventProcess<DiscoveryOut> for Behaviour<B, Client>
where
	B: BlockT,
	Client: HeaderBackend<B> + 'static,
{
	fn inject_event(&mut self, out: DiscoveryOut) {
		match out {
			DiscoveryOut::UnroutablePeer(_peer_id) => {
				// Obtaining and reporting listen addresses for unroutable peers back
				// to Kademlia is handled by the `Identify` protocol, part of the
				// `PeerInfoBehaviour`. See the `NetworkBehaviourEventProcess`
				// implementation for `PeerInfoEvent`.
			},
			DiscoveryOut::Discovered(peer_id) => {
				self.substrate.add_default_set_discovered_nodes(iter::once(peer_id));
			},
			DiscoveryOut::ValueFound(results, duration) => {
				self.events
					.push_back(BehaviourOut::Dht(DhtEvent::ValueFound(results), duration));
			},
			DiscoveryOut::ValueNotFound(key, duration) => {
				self.events.push_back(BehaviourOut::Dht(DhtEvent::ValueNotFound(key), duration));
			},
			DiscoveryOut::ValuePut(key, duration) => {
				self.events.push_back(BehaviourOut::Dht(DhtEvent::ValuePut(key), duration));
			},
			DiscoveryOut::ValuePutFailed(key, duration) => {
				self.events
					.push_back(BehaviourOut::Dht(DhtEvent::ValuePutFailed(key), duration));
			},
			DiscoveryOut::RandomKademliaStarted(protocols) =>
				for protocol in protocols {
					self.events.push_back(BehaviourOut::RandomKademliaStarted(protocol));
				},
		}
	}
}

impl<B, Client> Behaviour<B, Client>
where
	B: BlockT,
	Client: HeaderBackend<B> + 'static,
{
	fn poll(
		&mut self,
		_cx: &mut Context,
		_: &mut impl PollParameters,
	) -> Poll<NetworkBehaviourAction<BehaviourOut<B>, <Self as NetworkBehaviour>::ConnectionHandler>>
	{
		if let Some(event) = self.events.pop_front() {
			return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event))
		}

		Poll::Pending
	}
}
