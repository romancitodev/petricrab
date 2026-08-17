//! Small deterministic text format for authoring a [`PetriNet`] by hand: a `[places]` section
//! and a `[transitions]` section, one declaration per line, no nesting, no expressions. Meant
//! to mirror how a Petri net exercise is usually already written (a list of places, a list of
//! events with their pre/post-conditions), so transcribing one into this format is closer to
//! copying than translating into an unfamiliar schema.
//!
//! ```text
//! [places]
//! "pedido esperando M1" <a>
//! "M1 inactiva" <d> tokens=1
//!
//! [transitions]
//! "F1 inicia M1" <2>: @a, @d -> @i
//! ```

use std::collections::HashMap;

use crate::model::{ArcKind, PetriNet, PlaceId};

#[derive(Debug, Clone, PartialEq)]
pub struct DslError {
  pub line: usize,
  pub message: String,
}

fn err(line: usize, message: impl Into<String>) -> DslError {
  DslError {
    line,
    message: message.into(),
  }
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
  None,
  Places,
  Transitions,
}

struct RawPlace {
  line: usize,
  id: String,
  label: Option<String>,
  tokens: u32,
}

struct RawTransition {
  line: usize,
  id: String,
  label: Option<String>,
  inputs: Vec<(String, ArcKind)>,
  outputs: Vec<(String, u32)>,
}

/// Parses `source` into a fresh [`PetriNet`]. Collects every error in the document instead of
/// stopping at the first one — the text is usually pasted in bulk, so the point is fixing
/// everything in one pass. Never returns a partially-built net on error.
pub fn parse(source: &str) -> Result<PetriNet, Vec<DslError>> {
  let mut section = Section::None;
  let mut raw_places = Vec::new();
  let mut raw_transitions = Vec::new();
  let mut errors = Vec::new();

  for (i, raw_line) in source.lines().enumerate() {
    let line_no = i + 1;
    let line = strip_comment(raw_line).trim();
    if line.is_empty() {
      continue;
    }
    if line == "[places]" {
      section = Section::Places;
      continue;
    }
    if line == "[transitions]" {
      section = Section::Transitions;
      continue;
    }
    match section {
      Section::None => errors.push(err(line_no, "contenido antes de [places] o [transitions]")),
      // `ponytail:` a `:` inside a place's `"label"` would false-positive here too — same
      // corner-cut as `strip_comment`, fine while labels don't contain `:`.
      Section::Places if line.contains(':') => errors.push(err(
        line_no,
        "forma de transición dentro de [places] (¿falta el header [transitions]?)",
      )),
      Section::Places => match parse_place_line(line_no, line) {
        Ok(p) => raw_places.push(p),
        Err(e) => errors.push(e),
      },
      Section::Transitions if !line.contains(':') => errors.push(err(
        line_no,
        "falta ':' (forma de lugar dentro de [transitions])",
      )),
      Section::Transitions => match parse_transition_line(line_no, line) {
        Ok(t) => raw_transitions.push(t),
        Err(e) => errors.push(e),
      },
    }
  }

  let mut net = PetriNet::new();
  let mut place_ids: HashMap<String, PlaceId> = HashMap::new();
  for p in &raw_places {
    if place_ids.contains_key(&p.id) {
      errors.push(err(
        p.line,
        format!("lugar \"{}\" declarado más de una vez", p.id),
      ));
      continue;
    }
    let label = p.label.clone().unwrap_or_else(|| p.id.clone());
    let id = net.add_place(label);
    net.set_tokens(id, p.tokens);
    place_ids.insert(p.id.clone(), id);
  }

  for t in &raw_transitions {
    let label = t.label.clone().unwrap_or_else(|| t.id.clone());
    let tid = net.add_transition(label);
    for (place_id, kind) in &t.inputs {
      match place_ids.get(place_id) {
        Some(&p) => {
          if net.add_arc_place_to_transition(p, tid, *kind).is_err() {
            errors.push(err(
              t.line,
              format!("lugar \"{place_id}\" repetido como entrada"),
            ));
          }
        }
        None => errors.push(err(t.line, format!("lugar desconocido \"{place_id}\""))),
      }
    }
    for (place_id, weight) in &t.outputs {
      match place_ids.get(place_id) {
        Some(&p) => {
          if net.add_arc_transition_to_place(tid, p, *weight).is_err() {
            errors.push(err(
              t.line,
              format!("lugar \"{place_id}\" repetido como salida"),
            ));
          }
        }
        None => errors.push(err(t.line, format!("lugar desconocido \"{place_id}\""))),
      }
    }
  }

  if errors.is_empty() {
    Ok(net)
  } else {
    errors.sort_by_key(|e| e.line);
    Err(errors)
  }
}

/// `ponytail:` doesn't track quote state, so a `#` inside a `"quoted label"` truncates it too —
/// fine while labels in practice don't contain `#`; upgrade path: scan with in-quote tracking.
fn strip_comment(line: &str) -> &str {
  match line.find('#') {
    Some(idx) => &line[..idx],
    None => line,
  }
}

/// `["label"] <id>` — the shape shared by the head of a place line and the head of a transition
/// line (before its `:`). Returns the label (if quoted), the id (whatever's between `<` and
/// `>`), and whatever's left in `s` after the closing `>`.
fn parse_label_and_id(line_no: usize, s: &str) -> Result<(Option<String>, String, &str), DslError> {
  let mut rest = s.trim_start();
  let mut label = None;
  if let Some(after_quote) = rest.strip_prefix('"') {
    let end = after_quote
      .find('"')
      .ok_or_else(|| err(line_no, "comilla sin cerrar"))?;
    label = Some(after_quote[..end].to_string());
    rest = after_quote[end + 1..].trim_start();
  }

  let after_lt = rest
    .strip_prefix('<')
    .ok_or_else(|| err(line_no, "falta '<id>'"))?;
  let close = after_lt
    .find('>')
    .ok_or_else(|| err(line_no, "falta '>' después del id"))?;
  let id = after_lt[..close].trim();
  if id.is_empty() {
    return Err(err(line_no, "id vacío en <...>"));
  }

  Ok((label, id.to_string(), after_lt[close + 1..].trim()))
}

fn parse_place_line(line_no: usize, line: &str) -> Result<RawPlace, DslError> {
  let (label, id, rest) = parse_label_and_id(line_no, line)?;

  let mut tokens = 0u32;
  if !rest.is_empty() {
    let value = rest
      .strip_prefix("tokens=")
      .ok_or_else(|| err(line_no, format!("texto inesperado \"{rest}\"")))?;
    tokens = value
      .trim()
      .parse::<u32>()
      .map_err(|_| err(line_no, format!("tokens inválido \"{value}\"")))?;
  }

  Ok(RawPlace {
    line: line_no,
    id,
    label,
    tokens,
  })
}

fn parse_transition_line(line_no: usize, line: &str) -> Result<RawTransition, DslError> {
  let Some(colon) = line.find(':') else {
    return Err(err(line_no, "falta ':' antes de las entradas"));
  };
  let head = line[..colon].trim();
  let tail = &line[colon + 1..];

  let (label, id, rest) = parse_label_and_id(line_no, head)?;
  if !rest.is_empty() {
    return Err(err(
      line_no,
      format!("texto inesperado \"{rest}\" antes de ':'"),
    ));
  }

  let mut arrow_parts = tail.split("->");
  let ins_str = arrow_parts.next().unwrap_or("");
  let Some(outs_str) = arrow_parts.next() else {
    return Err(err(line_no, "falta '->'"));
  };
  if arrow_parts.next().is_some() {
    return Err(err(line_no, "más de un '->' en la línea"));
  }

  let inputs = parse_input_list(line_no, ins_str)?;
  let outputs = parse_output_list(line_no, outs_str)?;

  Ok(RawTransition {
    line: line_no,
    id,
    label,
    inputs,
    outputs,
  })
}

fn parse_list_items(line_no: usize, s: &str) -> Result<Vec<String>, DslError> {
  let s = s.trim();
  if s.is_empty() {
    return Ok(Vec::new());
  }
  let mut items = Vec::new();
  for part in s.split(',') {
    let part = part.trim();
    if part.is_empty() {
      return Err(err(line_no, "elemento vacío en la lista"));
    }
    items.push(part.to_string());
  }
  Ok(items)
}

/// Strips the `@` every place reference in a transition's input/output list must start with.
fn strip_at(line_no: usize, token: &str) -> Result<&str, DslError> {
  token
    .strip_prefix('@')
    .ok_or_else(|| err(line_no, format!("falta '@' antes de \"{token}\"")))
}

/// Splits a token like `p`, `p*2`, `p~`, `p!3` into its place id and an optional
/// `(modifier char, number string)` suffix.
fn split_modifier(token: &str) -> (&str, Option<(char, &str)>) {
  match token.find(['*', '~', '!']) {
    Some(idx) => (
      &token[..idx],
      Some((token.as_bytes()[idx] as char, &token[idx + 1..])),
    ),
    None => (token, None),
  }
}

fn parse_weight(line_no: usize, num_str: &str) -> Result<u32, DslError> {
  if num_str.is_empty() {
    return Ok(1);
  }
  num_str
    .parse::<u32>()
    .map_err(|_| err(line_no, format!("peso inválido \"{num_str}\"")))
}

fn parse_input_list(line_no: usize, s: &str) -> Result<Vec<(String, ArcKind)>, DslError> {
  parse_list_items(line_no, s)?
    .into_iter()
    .map(|token| {
      let (id, modifier) = split_modifier(strip_at(line_no, &token)?);
      let kind = match modifier {
        None => ArcKind::Consume(1),
        Some(('*', n)) => ArcKind::Consume(parse_weight(line_no, n)?),
        Some(('~', n)) => ArcKind::Peek(parse_weight(line_no, n)?),
        Some(('!', n)) => ArcKind::Inhibit(parse_weight(line_no, n)?),
        Some(_) => unreachable!("split_modifier only ever returns '*'/'~'/'!'"),
      };
      Ok((id.to_string(), kind))
    })
    .collect()
}

fn parse_output_list(line_no: usize, s: &str) -> Result<Vec<(String, u32)>, DslError> {
  parse_list_items(line_no, s)?
    .into_iter()
    .map(|token| {
      let (id, modifier) = split_modifier(strip_at(line_no, &token)?);
      let weight = match modifier {
        None => 1,
        Some(('*', n)) => parse_weight(line_no, n)?,
        Some((c, _)) => {
          return Err(err(
            line_no,
            format!("modificador '{c}' inválido en salida"),
          ));
        }
      };
      Ok((id.to_string(), weight))
    })
    .collect()
}

/// A DSL id must survive round-tripping unescaped inside `<...>` — the only character that
/// would break that is `>` itself.
fn is_valid_id(s: &str) -> bool {
  !s.is_empty() && !s.contains('>')
}

/// Serializes `net` back to DSL source. Round-trips losslessly (same place/transition count,
/// same inputs/outputs shape and weights) — used to seed a brand-new project's DSL buffer and
/// to preview a net built by hand on the canvas.
pub fn to_dsl(net: &PetriNet) -> String {
  let mut out = String::from("[places]\n");
  let mut place_id_of: HashMap<PlaceId, String> = HashMap::new();
  for (i, p) in net.place_ids().enumerate() {
    let label = net.place_label(p);
    let id = if is_valid_id(label) {
      label.to_string()
    } else {
      format!("p{i}")
    };
    place_id_of.insert(p, id.clone());
    if id != label {
      out.push_str(&format!("\"{label}\" "));
    }
    out.push_str(&format!("<{id}>"));
    let tokens = net.tokens(p);
    if tokens != 0 {
      out.push_str(&format!(" tokens={tokens}"));
    }
    out.push('\n');
  }

  out.push_str("\n[transitions]\n");
  for (i, t) in net.transition_ids().enumerate() {
    let label = net.transition_label(t);
    let id = if is_valid_id(label) {
      label.to_string()
    } else {
      format!("t{i}")
    };
    if id != label {
      out.push_str(&format!("\"{label}\" "));
    }
    out.push_str(&format!("<{id}>: "));

    let ins: Vec<String> = net
      .inputs(t)
      .iter()
      .map(|&(p, kind)| {
        let pid = &place_id_of[&p];
        match kind {
          ArcKind::Consume(1) => format!("@{pid}"),
          ArcKind::Consume(w) => format!("@{pid}*{w}"),
          ArcKind::Peek(1) => format!("@{pid}~"),
          ArcKind::Peek(w) => format!("@{pid}~{w}"),
          ArcKind::Inhibit(1) => format!("@{pid}!"),
          ArcKind::Inhibit(w) => format!("@{pid}!{w}"),
        }
      })
      .collect();
    out.push_str(&ins.join(", "));
    out.push_str(" -> ");

    let outs: Vec<String> = net
      .outputs(t)
      .iter()
      .map(|&(p, w)| {
        let pid = &place_id_of[&p];
        if w == 1 {
          format!("@{pid}")
        } else {
          format!("@{pid}*{w}")
        }
      })
      .collect();
    out.push_str(&outs.join(", "));
    out.push('\n');
  }

  out
}

/// The pre/post-condition table as Markdown — meant to be pasted straight into a report.
pub fn to_markdown(net: &PetriNet) -> String {
  let mut out = String::from("| Transición | Entradas (pre) | Salidas (post) |\n");
  out.push_str("| --- | --- | --- |\n");
  for t in net.transition_ids() {
    let ins = net
      .inputs(t)
      .iter()
      .map(|&(p, _)| net.place_label(p))
      .collect::<Vec<_>>()
      .join(", ");
    let outs = net
      .outputs(t)
      .iter()
      .map(|&(p, _)| net.place_label(p))
      .collect::<Vec<_>>()
      .join(", ");
    out.push_str(&format!(
      "| {} | {} | {} |\n",
      net.transition_label(t),
      if ins.is_empty() { "—" } else { &ins },
      if outs.is_empty() { "—" } else { &outs },
    ));
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  /// `Result::unwrap_err` needs the `Ok` side to be `Debug` (for its panic message), which
  /// `PetriNet` deliberately isn't — this sidesteps that instead of adding a derive nothing
  /// else needs.
  fn expect_err(source: &str) -> Vec<DslError> {
    match parse(source) {
      Ok(_) => panic!("expected a parse error"),
      Err(errors) => errors,
    }
  }

  #[test]
  fn happy_path_parses_places_and_transitions() {
    let source = r#"
[places]
"pedido esperando M1" <a>
"M1 inactiva" <d> tokens=1
"M1 operada por F1" <i>

[transitions]
"llega un pedido" <1>: -> @a
"F1 inicia M1" <2>: @a, @d -> @i
"entrega" <10>: @i ->
"#;
    let net = parse(source).unwrap();
    assert_eq!(net.place_ids().count(), 3);
    assert_eq!(net.transition_ids().count(), 3);

    let t2 = net
      .transition_ids()
      .find(|&t| net.transition_label(t) == "F1 inicia M1")
      .unwrap();
    assert_eq!(net.inputs(t2).len(), 2);
    assert_eq!(net.outputs(t2).len(), 1);

    let source_t = net
      .transition_ids()
      .find(|&t| net.transition_label(t) == "llega un pedido")
      .unwrap();
    assert!(net.inputs(source_t).is_empty());

    let sink_t = net
      .transition_ids()
      .find(|&t| net.transition_label(t) == "entrega")
      .unwrap();
    assert!(net.outputs(sink_t).is_empty());
  }

  #[test]
  fn unknown_place_id_reports_error_with_line_number() {
    let source = "[places]\n<a>\n\n[transitions]\n<t>: @a, @z -> @a\n";
    let errors = expect_err(source);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 5);
    assert!(errors[0].message.contains('z'));
  }

  #[test]
  fn content_before_any_section_header_is_error() {
    let source = "a\n[places]\n<a>\n";
    let errors = expect_err(source);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 1);
  }

  #[test]
  fn transition_shaped_line_inside_places_is_error() {
    let source = "[places]\n<t1>: @a -> @b\n";
    let errors = expect_err(source);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 2);
  }

  #[test]
  fn duplicate_place_id_is_error() {
    let source = "[places]\n<a>\n<a>\n";
    let errors = expect_err(source);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 3);
  }

  #[test]
  fn place_without_angle_id_is_error() {
    let source = "[places]\na\n";
    let errors = expect_err(source);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("<id>"));
  }

  #[test]
  fn input_without_at_prefix_is_error() {
    let source = "[places]\n<a>\n\n[transitions]\n<t>: a -> \n";
    let errors = expect_err(source);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains('@'));
  }

  #[test]
  fn input_modifiers_parse_weight_peek_inhibit() {
    let source = "[places]\n<p1>\n<p2>\n<p3>\n\n[transitions]\n<t>: @p1*2, @p2~, @p3!3 -> \n";
    let net = parse(source).unwrap();
    let t = net.transition_ids().next().unwrap();
    let inputs: HashMap<_, _> = net.inputs(t).iter().copied().collect();
    let labels: HashMap<&str, ArcKind> = inputs
      .iter()
      .map(|(&p, &k)| (net.place_label(p), k))
      .collect();
    assert_eq!(labels["p1"], ArcKind::Consume(2));
    assert_eq!(labels["p2"], ArcKind::Peek(1));
    assert_eq!(labels["p3"], ArcKind::Inhibit(3));
  }

  #[test]
  fn output_weight_modifier_parses() {
    let source = "[places]\n<p>\n<q>\n\n[transitions]\n<t>: @p -> @q*3\n";
    let net = parse(source).unwrap();
    let t = net.transition_ids().next().unwrap();
    assert_eq!(net.outputs(t)[0].1, 3);
  }

  #[test]
  fn errors_across_document_are_all_collected_not_just_first() {
    let source = "[places]\n<a>\n\n[transitions]\n<t1>: @a, @z -> @a\n<t2>: @a, @w -> @a\n";
    let errors = expect_err(source);
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].line, 5);
    assert_eq!(errors[1].line, 6);
  }

  #[test]
  fn to_dsl_then_parse_round_trips_structure() {
    let mut net = PetriNet::new();
    let p1 = net.add_place("p1");
    let p2 = net.add_place("p2");
    net.set_tokens(p1, 4);
    let t = net.add_transition("t1");
    net
      .add_arc_place_to_transition(p1, t, ArcKind::Peek(2))
      .unwrap();
    net.add_arc_transition_to_place(t, p2, 3).unwrap();

    let source = to_dsl(&net);
    let reparsed = parse(&source).unwrap();

    assert_eq!(reparsed.place_ids().count(), net.place_ids().count());
    assert_eq!(
      reparsed.transition_ids().count(),
      net.transition_ids().count()
    );
    let rt = reparsed.transition_ids().next().unwrap();
    assert_eq!(
      reparsed.inputs(rt),
      &[(reparsed_place(&reparsed, "p1"), ArcKind::Peek(2))]
    );
    assert_eq!(
      reparsed.outputs(rt),
      &[(reparsed_place(&reparsed, "p2"), 3)]
    );
    assert_eq!(reparsed.tokens(reparsed_place(&reparsed, "p1")), 4);
  }

  fn reparsed_place(net: &PetriNet, label: &str) -> PlaceId {
    net
      .place_ids()
      .find(|&p| net.place_label(p) == label)
      .unwrap()
  }
}
