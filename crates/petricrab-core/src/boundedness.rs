#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boundedness {
  /// Max *k* tokens seen.
  Bounded(usize),
  Unbounded,
}
