// Types for serde.ts, the fixture `cargo test` writes from the Rust serde shapes. tsc checks that
// file against types.ts on every typecheck and build.

/** T with every optional field required, all the way down. The fixture sets every Rust Option, so a
 *  field types.ts declares that Rust never sends shows up as missing. */
export type Full<T> = T extends object ? { [K in keyof T]-?: Full<T[K]> } : T

/** `true` when A and B hold the same values; otherwise names the values only one side has. */
export type Same<A, B> = [Exclude<A, B> | Exclude<B, A>] extends [never] ? true : { onlyOnOneSide: Exclude<A, B> | Exclude<B, A> }
