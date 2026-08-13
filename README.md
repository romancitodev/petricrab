<div align="center">

# `🦀 petricrab`

**Librería en Rust para modelar y simular [redes de Petri](https://es.wikipedia.org/wiki/Red_de_Petri)**

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/romancitodev/petricrab/workflows/CI/badge.svg)](https://github.com/romancitodev/petricrab/actions?workflow=CI)

</div>

## Crates

| Crate | Descripción |
| --- | --- |
| [`petricrab-core`](crates/petricrab-core) | Modelo de red de Petri: lugares, transiciones y arcos con peso, soportando arcos de consumo, peek (lectura sin consumir) e inhibición. Habilitación/disparo de transiciones, grafo de alcanzabilidad y grafo de cobertura de Karp-Miller (termina incluso en redes no acotadas). Análisis: acotamiento/safety, liveness (niveles L0–L4 de Murata) y reversibilidad/home states. |
| [`petricrab-app`](crates/petricrab-app) | Editor visual de escritorio con [egui](https://github.com/emilk/egui)/eframe: canvas con pan/zoom, simulador paso a paso, paneles dockeables (grafo de alcanzabilidad, propiedades, outline, selección), guardado/carga de proyectos `.gpn` y tema claro/oscuro. |

## Estructura

| Ruta | Descripción |
| --- | --- |
| `src/` | Binario principal |
| `crates/petricrab-core/` | Núcleo: modelo, simulación y análisis de redes de Petri |
| `crates/petricrab-app/` | Editor visual (eframe/egui) |

## Editor (`petricrab-app`)

| Área | Qué hace |
| --- | --- |
| Canvas | Pan/zoom, grid, arcos de consumo/peek/inhibit, rotación de transiciones, notas de texto libres |
| Simulación | Token game paso a paso, deshacer/rehacer, reset al marking inicial |
| Análisis | Grafo de alcanzabilidad interactivo (pan/zoom y ajustar-a-vista propios) y panel de propiedades (acotamiento, liveness, reversibilidad), en paneles dockeables junto al canvas |
| Proyectos | Guardar/abrir `.gpn` (formato binario propio), lista de recientes persistida |
| Tema | Claro/oscuro, persistido entre sesiones |

## Roadmap

| Feature | Estado |
| --- | --- |
| Simulación completa de una red de Petri | ✅ Completo |
| Análisis: acotamiento, liveness, reversibilidad/home states | ✅ Completo |
| `petricrab-app` — editor visual con [egui](https://github.com/emilk/egui) | 🚧 En progreso (pulido de UI) |
| Arcos peek/inhibit con peso configurable en `petricrab-core` | 📋 Planeado |
| Funciones helper de optimización | 📋 Planeado |

## Referencias

- [Paper](http://people.disim.univaq.it/adimarco/teaching/bioinfo15/paper.pdf) en el que se basa `petricrab-core`.

## Licencia

Licenciado bajo [MIT](LICENSE).
