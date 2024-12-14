use async_std::sync::{RwLock, RwLockReadGuard};
use async_trait::async_trait;
use futures::{future, select};
use futures::FutureExt;
use std::{
  collections::{HashMap, HashSet, VecDeque},
  net::IpAddr,
};
use uuid::Uuid;

use crate::{
  core::{MessageServer, SpamChecker, MAILBOX_SIZE},
  messages::{
    ClientError, ClientId, ClientMessage, ClientPollReply, ClientReply, FullyQualifiedMessage,
    Sequence, ServerId,
  },
};

use crate::messages::ClientError::BoxFull;
use crate::messages::ClientMessage::MText;
use crate::messages::ClientPollReply::{Message, Nothing};
use crate::messages::ClientReply::{Delayed, Delivered, Error, Transfer};
use crate::messages::ServerReply::EmptyRoute;
use crate::messages::{Outgoing, ServerMessage, ServerReply};

// this structure will contain the data you need to track in your server
// this will include things like delivered messages, clients last seen sequence number, etc.
pub struct Server<C: SpamChecker> {
  checker: C,
  server_id: ServerId,
  client_list: RwLock<HashMap<ClientId, ClientInfo>>,
  routes: RwLock<HashMap<ServerId, Vec<ServerId>>>,
  stored_message: RwLock<HashMap<ClientId, Vec<Mail>>>,
  remote_clients: RwLock<HashMap<ClientId, RemoteClientInfo>>,
}

pub struct ClientInfo {
  src_ip: IpAddr,
  name: String,
  sequence_id: u128,
  mail_box: VecDeque<Mail>,
}

pub struct Mail {
  content: String,
  src: ClientId,
}

struct RemoteClientInfo {
  name: String,
  srv: ServerId,
}

#[async_trait]
impl<C: SpamChecker + Send + Sync> MessageServer<C> for Server<C> {
  const GROUP_NAME: &'static str = "SERVET-BOUDOU Isaure, SOLA Maxime";

  fn new(checker: C, id: ServerId) -> Self {
    Server {
      checker,
      server_id: id,
      client_list: RwLock::new(HashMap::new()),
      routes: Default::default(),
      stored_message: RwLock::new(HashMap::new()),
      remote_clients: Default::default(),
    }
  }

  // note: you need to roll a Uuid, and then convert it into a ClientId
  // Uuid::new_v4() will generate such a value
  // you will most likely have to edit the Server struct as as to store information about the client
  //
  // for spam checking, you will need to run both checks in parallel, and take a decision as soon as
  // each checks return

  async fn register_local_client(&self, src_ip: IpAddr, name: String) -> Option<ClientId> {
    let name_clone = name.clone();
    let mut is_user_spammer = self.checker.is_user_spammer(&name_clone).fuse();
    let mut is_ip_spammer = self.checker.is_ip_spammer(&src_ip).fuse();

    loop {
      select! {
        usr = is_user_spammer => if usr { return None },
        ip = is_ip_spammer => if ip { return None },
        complete => break,
        default => unreachable!()
      }
    }

    let new_client_info = ClientInfo {
      src_ip,
      name,
      sequence_id: 0,
      mail_box: VecDeque::new(),
    };
    let id_client = ClientId(Uuid::new_v4());
    self
      .client_list
      .write()
      .await
      .insert(id_client, new_client_info);
    Some(id_client)
    //un jour il y aura none
  }

  /*
   if the client is known, its last seen sequence number must be verified (and updated)
  */
  async fn handle_sequenced_message<A: Send>(
    &self,
    sequence: Sequence<A>,
  ) -> Result<A, ClientError> {
    match self.client_list.write().await.get_mut(&sequence.src) {
      None => Err(ClientError::UnknownClient),
      Some(client) => {
        if client.sequence_id < sequence.seqid {
          client.sequence_id = sequence.seqid;
          Ok(sequence.content)
        } else {
          Err(ClientError::InternalError)
        }
      }
    }
  }

  /* Here client messages are handled.
    * if the client is local,
      * if the mailbox is full, BoxFull should be returned
      * otherwise, Delivered should be returned
    * if the client is unknown, the message should be stored and Delayed must be returned
    * (federation) if the client is remote, Transfer should be returned

    It is recommended to write an function that handles a single message and use it to handle
    both ClientMessage variants.
  */
  async fn handle_client_message(&self, src: ClientId, msg: ClientMessage) -> Vec<ClientReply> {
    match msg {
      ClientMessage::Text { dest, content } => {
        vec![self.helper(src, dest, content).await]
      }
      MText { dest, content } => {
        let mut new_vec: Vec<ClientReply> = Vec::new();
        for id in dest.iter() {
          new_vec.push(self.helper(src, *id, content.clone()).await)
        }
        new_vec
      }
    }
  }

  /* for the given client, return the next message or error if available
   */
  async fn client_poll(&self, client: ClientId) -> ClientPollReply {
    match self.client_list.write().await.get_mut(&client) {
      None => Nothing,
      Some(clients) => match clients.mail_box.pop_front() {
        None => Nothing,
        Some(pop_message) => ClientPollReply::Message {
          src: pop_message.src,
          content: pop_message.content,
        },
      },
    }
  }

  /* For announces
     * if the route is empty, return EmptyRoute
     * if not, store the route in some way
     * also store the remote clients
     * if one of these remote clients has messages waiting, return them
    For messages
     * if local, deliver them
     * if remote, forward them
  */

  async fn handle_server_message(&self, msg: ServerMessage) -> ServerReply {
    match msg {
      ServerMessage::Announce { route, clients } => {
        let mut write_route = self.routes.write().await;
        match route.last() {
          None => EmptyRoute,
          Some(&dest_server) => {
            write_route.insert(dest_server, route.clone());
            let mut result: Vec<Outgoing<FullyQualifiedMessage>> = vec![];
            let mut writable_delayed_box = self.stored_message.write().await;
            let mut writable_remote_clients = self.remote_clients.write().await;
            for (client_id, name) in clients.iter() {
              writable_remote_clients.insert(
                *client_id,
                RemoteClientInfo {
                  srv: dest_server,
                  name: name.clone(),
                },
              );
              match writable_delayed_box.get(client_id) {
                Some(delayed_messages) => {
                  for delayed_msg in delayed_messages.iter() {
                    result.push(Outgoing {
                      nexthop: dest_server,
                      message: FullyQualifiedMessage {
                        src: delayed_msg.src,
                        srcsrv: self.server_id,
                        dsts: vec![(*client_id, *route.first().unwrap())],
                        content: delayed_msg.content.clone(),
                      },
                    })
                  }
                  writable_delayed_box.remove(client_id);
                }
                None => continue,
              }
            }
            ServerReply::Outgoing(result)
          }
        }
      }

      ServerMessage::Message(msgServ) => {

        for id in msgServ.dsts.iter() {
          if msgServ.srcsrv == self.server_id {
            let contenu: ClientMessage = ClientMessage::Text {
              dest: id.0,
              content: msgServ.content.clone(),
            };
            self.handle_client_message(msgServ.src, contenu).await;
          } else {
            let transfer_message: FullyQualifiedMessage = msgServ.clone().into();
            let contenu: ServerMessage = ServerMessage::Message {
              0: transfer_message
            };
            for (client_id, server_id) in msgServ.dsts.iter() {
              Transfer(server_id.clone(), contenu.clone());
            }
          }
        }
        todo!()
      }
    }
  }

  /*
  pub struct FullyQualifiedMessage {
  pub src: ClientId,
  pub srcsrv: ServerId,
  pub dsts: Vec<(ClientId, ServerId)>,
  pub content: String,
   */

  async fn list_users(&self) -> HashMap<ClientId, String> {
    let mut map_client: HashMap<ClientId, String> = HashMap::new();
    for (id, info) in self.client_list.read().await.iter() {
      map_client.insert(*id, info.name.clone());
    }
    map_client
  }

  // return a route to the target server
  // bonus points if it is the shortest route
  async fn route_to(&self, destination: ServerId) -> Option<Vec<ServerId>> {
    let readable_server_routes = self.routes.read().await;
    readable_server_routes.get(&destination).cloned()
  }
}

impl<C: SpamChecker + Sync + Send> Server<C> {
  async fn helper(&self, src: ClientId, dest: ClientId, content: String) -> ClientReply {
    match self.client_list.write().await.get_mut(&dest) {
      None => {
        let readable_remote_clients = self.remote_clients.read().await;
        match readable_remote_clients.get(&dest) {
          Some(remote_client_info) => {
            let first_server = *self.route_to(remote_client_info.srv).await.unwrap().first().unwrap();
            ClientReply::Transfer(
              remote_client_info.srv,
              ServerMessage::Message(FullyQualifiedMessage {
                src,
                srcsrv: self.server_id,
                dsts: vec![(dest, first_server)],
                content,
              }),
            )
          }
          None => {
            let mut write_delayed_box = self.stored_message.write().await;
            let entry = write_delayed_box.entry(dest).or_insert(vec![]);
            entry.push(Mail { src, content });
            ClientReply::Delayed
          }
        }
      }
      Some(info) => {
        if info.mail_box.len() < MAILBOX_SIZE {
          let mailbox = Mail { content, src };
          info.mail_box.push_back(mailbox);
          Delivered
        } else {
          Error(BoxFull(dest))
        }
      }
    }
  }
}

#[cfg(test)]
mod test {
  use crate::testing::{test_message_server, TestChecker};

  use super::*;

  #[test]
  fn tester() {
    test_message_server::<Server<TestChecker>>();
  }
}
