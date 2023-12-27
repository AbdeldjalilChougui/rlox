# rlox

The Lox language from Robert Nystrom's [*Crafting Interpreters*](https://craftinginterpreters.com),
implemented twice in Rust:

1. **Tree-walk interpreter** (`src/tree/`, the book's "jlox"): parser → AST → resolver → evaluator.
2. **Bytecode VM** (`src/vm/`, the book's "clox"): single-pass Pratt compiler → bytecode → stack VM.

Both support the full language: variables, blocks, `if`/`while`/`for`, functions, closures,
classes, fields, methods, initializers, `this`, inheritance and `super`, plus the `clock()` native.

## Quick start

```sh
cargo build --release
./target/release/rlox                    # REPL (bytecode VM)
./target/release/rlox --tree             # REPL (tree-walker)
./target/release/rlox script.lox         # run a file with the VM
./target/release/rlox --tree script.lox  # run a file with the tree-walker
```

Exit codes follow the book: `65` for compile errors, `70` for runtime errors.

## How it works

- `scanner.rs` is shared by both back ends.
- **Tree-walker:** `parser.rs` builds an AST (`for` is desugared into `while`), `resolver.rs`
  computes the scope depth of each local reference (so closures bind statically), and
  `interpreter.rs` evaluates with `Rc<RefCell<..>>` environments.
- **VM:** `compiler.rs` emits bytecode directly while parsing (locals live in stack slots,
  captured variables become upvalues), `machine.rs` runs it with call frames, open/closed
  upvalues, `Invoke`/`SuperInvoke` fast paths for method calls, and copy-down inheritance.

On `fib(27)` the VM is about 4x faster than the tree-walker (release build).

## Tests

```sh
cargo test
```

`tests/lox/*.lox` is a suite of Lox programs annotated with `// expect: ...`,
`// expect runtime error: ...` and `// expect compile error: ...`. `tests/lox_suite.rs`
runs every file against **both** back ends and checks stdout, stderr and the exit code.
A REPL test checks that state persists across lines.

## Status / limitations

Learning project.
- The VM uses reference counting instead of the book's mark-and-sweep GC, so reference
  cycles (e.g. an object holding a closure that captures it) leak memory.
- No string interning; strings compare by content.
- Max 256 constants per function, 256 locals, 256 call frames (VM) / 1000 calls (tree-walker).
- The test suite is a hand-written subset, not the official Crafting Interpreters suite.

## License

MIT, see [LICENSE](LICENSE).
