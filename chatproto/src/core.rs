use std::collections::HashMap;
use std::net::IpAddr;

use async_trait::async_trait;

use crate::messages::{
  ClientError, ClientId, ClientMessage, ClientPollReply, ClientReply, Sequence, ServerId,
};
use crate::messages::{ServerMessage, ServerReply};

pub const MAILBOX_SIZE: usize = 256;

#[async_trait]
pub trait SpamChecker {
  async fn is_user_spammer(&self, name: &str) -> bool;
  async fn is_ip_spammer(&self, name: &IpAddr) -> bool;
}

#[async_trait]
pub trait MessageServer<C: SpamChecker> {
  /// group name
  const GROUP_NAME: &'static str;

  /// create a new server, this is the constructor function
  fn new(checker: C, id: ServerId) -> Self;

  /// register a new client, that will then be able to send and receive messages.
  /// The first argument is the client screen name.
  ///
  /// if any of the spam check fails, you should return None and not register the client.
  async fn register_local_client(&self, src_ip: IpAddr, name: String) -> Option<ClientId>;

  /// list known users
  /// also lists known remote users if federation is enabled
  async fn list_users(&self) -> HashMap<ClientId, String>;

  /// handles a sequenced message
  /// you must verify that sequence numbers are increasing
  async fn handle_sequenced_message<A: Send>(&self, msg: Sequence<A>) -> Result<A, ClientError>;

  /// pull function for the client
  async fn client_poll(&self, client: ClientId) -> ClientPollReply;

  /// handles a client message
  /// * if the user is unknown, it might be that it is remote, so messages should be kept until the user becomes known
  ///   as a result, the "Delayed" message should be sent
  /// * until polled, messages are to be stored. There is a maximum mailbox size after which an error should be returned
  async fn handle_client_message(&self, src: ClientId, msg: ClientMessage) -> Vec<ClientReply>;

  /// handles a server message
  /// * might be an announce (which might trigger waiting messages to be sent)
  /// * might be a message for this server, or another
  async fn handle_server_message(&self, msg: ServerMessage) -> ServerReply;

  /// gives the best route to a server
  /// as a first approximation, you can give any route
  async fn route_to(&self, destination: ServerId) -> Option<Vec<ServerId>>;
}

// a spam checker that does nothing
#[derive(Clone, Copy, Default)]
pub struct DefaultChecker {}

#[async_trait]
impl SpamChecker for DefaultChecker {
  async fn is_user_spammer(&self, _name: &str) -> bool {
    false
  }
  async fn is_ip_spammer(&self, _name: &IpAddr) -> bool {
    false
  }
}
