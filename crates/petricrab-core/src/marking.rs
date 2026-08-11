use crate::net::PlaceId;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Marking(pub(crate) Vec<usize>);

#[expect(
  dead_code,
  reason = "We are not using this struct yet, but we will use it in the future."
)]
pub struct MarkingFixed(Vec<usize>, usize);

impl std::fmt::Debug for Marking {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "M{{")?;
    let mut first = true;
    for (i, &tokens) in self.0.iter().enumerate() {
      if tokens == 0 {
        continue;
      }
      if !first {
        write!(f, ", ")?;
      }
      write!(f, "p{i}: {tokens}")?;
      first = false;
    }
    write!(f, "}}")
  }
}

impl Marking {
  #[must_use = "This method does not modify the marking in place, it returns a new marking."]
  pub fn new(tokens: Vec<usize>) -> Self {
    Marking(tokens)
  }

  #[must_use = "This method does not modify the marking in place, it returns a new marking."]
  /// Get the number of tokens in a specific place
  pub fn tokens(&self, place_id: PlaceId) -> usize {
    self.0[place_id.0]
  }
}
