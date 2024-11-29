use crate::messages::{ClientId, Sequence};

#[derive(Debug, Default)]
pub struct Client {
  id: ClientId,
  curid: u128,
}

impl Client {
  pub fn new(id: ClientId) -> Self {
    Client { id, curid: 0 }
  }
  pub fn sequence<A>(&mut self, content: A) -> Sequence<A> {
    self.curid += 1;
    Sequence {
      seqid: self.curid,
      src: self.id,
      content,
    }
  }
}
