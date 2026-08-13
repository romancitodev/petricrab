# `petricrab-core`

Núcleo de [redes de Petri](https://es.wikipedia.org/wiki/Red_de_Petri) en Rust puro, sin dependencias externas. Modela la red, la simula, y analiza sus propiedades formales.

## Modelo

| Concepto | Tipo | Notas |
| --- | --- | --- |
| Lugar | `PlaceId` | Cuenta de tokens sin límite (capacidad infinita por diseño; ver `PetriNetFixed` para el caso acotado, todavía no usado) |
| Transición | `TransitionId` | Se dispara si todas sus entradas están satisfechas |
| Arco | `ArcKind` | `Consume(peso)`, `Peek` (lee sin consumir), `Inhibit` (exige cero tokens) |
| Marcado | `Marking` | Vector de tokens por lugar |

## Simulación

`PetriNet::enabled_transitions` y `Transition::fire` implementan la regla de disparo clásica. `PetriNet::reachable_markings` construye el grafo de alcanzabilidad exacto por BFS — nunca termina en una red no acotada, así que casi todo el análisis de acá para abajo tiene una variante `_covering` que corre sobre el grafo de cobertura de Karp-Miller (`coverability::coverability_graph`) en su lugar, siempre finito.

## Análisis

| Propiedad | Funciones | Descripción |
| --- | --- | --- |
| Acotamiento / safety | `boundedness_report` | `Bounded(k)` o `Unbounded` por lugar, sobre el grafo de cobertura |
| Liveness | `liveness_report`, `liveness_report_covering`, `liveness_of` | Niveles L0–L4 de Murata, con una secuencia de disparo testigo por transición |
| Reversibilidad | `is_reversible`, `is_reversible_covering`, `home_states` | Si siempre se puede volver al marcado inicial, y el conjunto de home states si no |
| Deadlocks | `deadlocks`, `deadlocks_covering` | Marcados sin transiciones habilitadas, con la secuencia de disparo más corta hasta cada uno |

Cada función `_covering` documenta su propio límite de precisión en el doc-comment (típicamente: exacta salvo por la distinción L2/L3 de Murata, o por arcos `Inhibit` sobre un lugar no acotado).

## Detección de deadlocks

`deadlocks` recorre el grafo de alcanzabilidad exacto buscando marcados sin transiciones habilitadas. Como ese grafo es infinito en una red no acotada, `deadlocks_covering` hace lo mismo sobre el grafo de cobertura de Karp-Miller (siempre termina).

`deadlocks_covering` es *sound* pero no completo: un nodo muerto en el grafo de cobertura siempre corresponde a un marcado realmente muerto, salvo que la red tenga arcos `Inhibit` sobre un lugar no acotado, ahí `Ω` puede hacer parecer deshabilitada una transición que en la ejecución concreta sí tendría el lugar en cero. Cerrar ese caso general requiere análisis estructural por *siphons* (Teorema del siphon vacío: en una red ordinaria, todo marcado muerto contiene un siphon vacío), que es NP-hard en el caso general y queda fuera de alcance por ahora.

## Referencias

- [Paper](http://people.disim.univaq.it/adimarco/teaching/bioinfo15/paper.pdf) en el que se basa este crate.
- [Deadlock analysis and control based on Petri nets: A siphon approach review](https://journals.sagepub.com/doi/10.1177/1687814017693542)
- [Deadlock analysis of Petri nets using siphons and mathematical programming](https://ieeexplore.ieee.org/document/650158/)
- [The Minimal Coverability Graph for Petri Nets](https://link.springer.com/chapter/10.1007/3-540-56689-9_45)
