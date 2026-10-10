---
disclaimer:
  notice: >-
    No information within this document should be taken for granted.
    Any statement or premise not backed by a real logical definition
    or verifiable reference may be invalid, erroneous, or a hallucination.
  generated_by: "AI-assisted; the model is named in the commit trailer"
  date: "2026-10-09"
---

# A maze from a program (SREP 66)

`maze.scene.xml` draws a 32 × 18 maze, then its solution. The geometry comes from `maze.wasm`, a WebAssembly
module that runs once when the document loads. No script writes the XML.

## How the module works

- **Maze.** It carves the maze by recursive backtracking, choosing every branch with `sr.rand_u64`, the seeded
  generator of SREP 66.
- **Solution.** It finds the path from the top-left corner to the bottom-right one by breadth-first search.
- **Output.** It returns two `shape="path"` nodes whose `trimEnd` draws them on: the walls, then the route.

## Changing the maze

- **Seed.** `program/@seed` picks the maze, and the same seed gives the same maze on every host.
- **Parameters.** `cols`, `rows`, `cell`, `x`, `y`, `wall`, `draw`, `solve` and `pause` are read with
  `sr.param_f64`. Their defaults are in `maze/src/lib.rs`.

## Rebuilding the module

Run `./build.sh`. It needs `rustup target add wasm32-unknown-unknown`, and it prints the new SHA-256 for
`program/@sha256`. The module imports only `sr.rand_u64` and `sr.param_f64`.

## Rendering

```sh
scene-render render maze.scene.xml -f 240 -o still.png
scene-render encode maze.scene.xml --output master
```
