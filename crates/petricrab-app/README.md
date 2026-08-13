# `petricrab-app`

Editor visual de escritorio para redes de Petri, construido con [egui](https://github.com/emilk/egui)/eframe sobre [`petricrab-core`](../petricrab-core).

## Canvas

Pan/zoom, grid, lugares y transiciones editables (arrastrar, rotar transiciones), arcos de consumo/peek/inhibit, y notas de texto libre.

**Notas**: se les puede cambiar el color desde el panel de Selección. Un click sobre una nota la selecciona (foco); un segundo click sobre la nota ya seleccionada recién ahí abre el texto para editar inline, así arrastrar una nota ya seleccionada no la mete en modo edición por accidente.

## Simulación

Token game paso a paso desde el popup flotante o clickeando una transición habilitada directamente en el canvas, con deshacer/rehacer y reset al marcado inicial.

## Análisis

Panel de propiedades (acotamiento, liveness, reversibilidad/home states, deadlocks) y grafo de alcanzabilidad interactivo, como paneles dockeables junto al canvas.

### Ver ruta

Cualquier resultado del análisis que trae una secuencia de disparo testigo (un deadlock, un ejemplo de liveness, el camino a un home state, o un nodo del grafo de alcanzabilidad) tiene un botón **"Ver ruta"** que la reproduce sobre el canvas real:

- El marcado del net se sobreescribe paso a paso con el de la ruta grabada, así los tokens se mueven sobre las posiciones reales que vos armaste, no un diagrama aparte.
- Todo lo que no es parte de la ruta queda atenuado. Los arcos de entrada de la transición que está por dispararse se pintan de naranja, los de salida de celeste, y los de transiciones ya disparadas quedan en verde.
- El canvas queda de solo lectura mientras tanto (nada de arrastrar, seleccionar ni editar), pero pan/zoom siguen libres para poder mirar alrededor.
- La cámara sigue el paso actual automáticamente.
- Controles: botón "Disparar" (o el aviso correspondiente si es un deadlock o si la ruta no se pudo reproducir 1:1 en una red no acotada), Atrás/Reiniciar/Cerrar, o ← / → / espacio para avanzar y retroceder, Esc para cerrar.

## Proyectos

Guardar/abrir `.gpn`, un formato binario propio ([rkyv](https://rkyv.org/)) con snapshot del net, posiciones, colores, rotaciones y notas. Versión de formato actual: 3 — un archivo `.gpn` de una versión distinta no carga (sin migración automática todavía). Lista de recientes persistida entre sesiones.

## Tema

Claro/oscuro, persistido entre sesiones.
