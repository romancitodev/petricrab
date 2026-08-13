# Changelog

Generado automáticamente con [git-cliff](https://git-cliff.org/) a partir de los mensajes de commit.
## [0.1.0] - 2026-08-13

### ♻️ Refactor

- Split lib.rs into marking/net/liveness modules (6f0cc94)

### ✨ Features

- Add initial Petri net model (c42bcc1)
- Build reachability graph and cycle detection (3d1cb4f)
- Add liveness_of and whole-net liveness_report (1bc8331)
- Add Transition::arcs accessor and PetriNet::place_ids (5a4e67f)
- Add Karp-Miller coverability graph (202af1e)
- Scaffold Boundedness enum (cad1ed3)
- Add boundedness_report and Boundedness::is_safe (91a6418)
- Add reversibility and home-state analysis (7e35f19)
- Add Weight::new public constructor (f55b232)
- Add eframe GUI editor, ported from the petri-nets reference (927cd3c)
- Populate LivenessReport::example with a real witness path (886dc9e)
- Compute liveness/reversibility over the coverability graph (031caf3)
- Surface partial analysis for unbounded nets (048d0f0)
- Editor overhaul — menus, context menus, canvas, navigation (d2ef02a)
- Editable names, .gpn project files, toast notifications (edca204)
- Dockable analysis panels, light/dark theme, monospace UI (a0708fa)
- Deadlock detection, route replay across analyses, note colors, release CI (a816152)

### 🎨 Style

- Give floating windows their own chrome (c30214b)
- Reformat to project's 2-space convention (96d8a4b)
- Shadcn-inspired dark theme (5240607)

### 🐛 Fixes

- Reaction oxygen consumption test (71ed3b3)
- Reachability window movable + themed graph + detail (acea140)

### 👷 CI

- Add GitHub Actions workflow with cargo test (066c825)

### 📝 Documentation

- Add README (b7059a0)
- Add crab emoji and CI badge to README (f2d0c63)
- Update README for petricrab-app and the analysis capabilities (7ccc9b9)
- Update README for dockable panels, .gpn projects, theme toggle (c2ce0b0)

### 🔧 Chores

- Initial workspace scaffold (13de069)
- Add MIT license (a96c3d9)
- Fix two clippy lints in coverability.rs (3ccfd25)
