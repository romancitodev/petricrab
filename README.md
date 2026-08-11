<div align="center">

# `🦀 petricrab`

**Librería en Rust para modelar y simular [redes de Petri](https://es.wikipedia.org/wiki/Red_de_Petri)**

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/romancitodev/petricrab/workflows/CI/badge.svg)](https://github.com/romancitodev/petricrab/actions?workflow=CI)

</div>

## Crates

| Crate | Descripción |
| --- | --- |
| [`petricrab-core`](crates/petricrab-core) | Modelo de red de Petri: lugares, transiciones y arcos con peso, soportando arcos de consumo, peek (lectura sin consumir) e inhibición. Incluye lógica de habilitación y disparo de transiciones. |

## Estructura

| Ruta | Descripción |
| --- | --- |
| `src/` | Binario principal |
| `crates/petricrab-core/` | Núcleo: modelo y lógica de redes de Petri |

## Roadmap

| Feature | Estado |
| --- | --- |
| Simulación completa de una red de Petri | 🚧 En progreso |
| Funciones helper de optimización | 📋 Planeado |
| `petricrab-gui` — editor visual con [egui](https://github.com/emilk/egui) (alcance por definir) | 📋 Planeado |

## Referencias

- [Paper](http://people.disim.univaq.it/adimarco/teaching/bioinfo15/paper.pdf) en el que se basa `petricrab-core`.

## Licencia

Licenciado bajo [MIT](LICENSE).
