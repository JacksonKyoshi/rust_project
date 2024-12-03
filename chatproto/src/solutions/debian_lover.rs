use async_std::sync::{RwLock, RwLockReadGuard};
use futures::{future, select};
use async_trait::async_trait;
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

use crate::messages::{Outgoing, ServerMessage, ServerReply};
use crate::messages::ClientError::BoxFull;
use crate::messages::ClientMessage::MText;
use crate::messages::ClientPollReply::Nothing;
use crate::messages::ClientReply::{Delayed, Delivered, Error, Transfer};

// this structure will contain the data you need to track in your server
// this will include things like delivered messages, clients last seen sequence number, etc.
pub struct Server<C: SpamChecker> {
  checker: C,
  server_id: ServerId,
  client_list: RwLock<HashMap<ClientId, ClientInfo>>,
  stored_message: RwLock<HashMap<ClientId,Vec<Mail>>>,
}

pub struct ClientInfo{
  src_ip: IpAddr,
  name: String,
  sequence_id: u128,
  mail_box: VecDeque<Mail>,
}

pub struct Mail {
  content: String,
  src: ClientId,
}

#[async_trait]
impl<C: SpamChecker + Send + Sync> MessageServer<C> for Server<C> {
  const GROUP_NAME: &'static str = "SERVET-BOUDOU Isaure, SOLA Maxime";

  fn new(checker: C, id: ServerId) -> Self {
    Server { checker, server_id: id, client_list: RwLock::new(HashMap::new()), stored_message: RwLock::new(HashMap::new()) }
  }

  // note: you need to roll a Uuid, and then convert it into a ClientId
  // Uuid::new_v4() will generate such a value
  // you will most likely have to edit the Server struct as as to store information about the client
  //
  // for spam checking, you will need to run both checks in parallel, and take a decision as soon as
  // each checks return
  async fn register_local_client(&self, src_ip: IpAddr, name: String) -> Option<ClientId> {
    let new_client_info = ClientInfo { src_ip, name, sequence_id: 0, mail_box: VecDeque::new() };
    let id_client = ClientId(Uuid::new_v4());
    self.client_list.write().await.insert(id_client, new_client_info);
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
      None => {
        Err(ClientError::UnknownClient)
      }
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
      ClientMessage::Text {dest,content} =>{
        vec![self.helper(dest, content).await]
      }
      MText {dest,content}=> {
        let mut new_vec : Vec<ClientReply> = Vec::new();
        for id in dest.iter() {
          new_vec.push(self.helper(*id, content.clone()).await)
        }
        new_vec
      }
    }
  }

  /* for the given client, return the next message or error if available
   */
  async fn client_poll(&self, client: ClientId) -> ClientPollReply {
    match self.client_list.read().await.get(&client) {
      None => {
        Nothing
      }
      Some(clients) => {
        
      }
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
    todo!()
  }

  async fn list_users(&self) -> HashMap<ClientId, String> {
    todo!()
  }

  // return a route to the target server
  // bonus points if it is the shortest route
  async fn route_to(&self, destination: ServerId) -> Option<Vec<ServerId>> {
    todo!()
  }
}

impl<C: SpamChecker + Sync + Send> Server<C> {
  async fn helper(&self, dest:ClientId, content:String) -> ClientReply {
    match self.client_list.write().await.get_mut(&dest) {
      None => {
        let mut truc = self.stored_message.write().await;
        //en gros ça va lire la hashMap pour savoir si l'id existe dedans, si il existe, on écrit le message dans le vec, si il n'existe pas, il écrit à la suite et créer le champ avec l'id
        let new_mail = Mail { content, src: dest };
        truc.entry(dest).or_default().push(new_mail);
        Delayed
      }
      Some(info) => {
        if info.mail_box.len() < MAILBOX_SIZE {
          let mailbox = Mail{content, src: dest};
          info.mail_box.push_back(mailbox);
          Delivered
        }
        else {
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
