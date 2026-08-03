[![`codas` on crates.io](https://img.shields.io/crates/v/codas)](https://crates.io/crates/codas)
[![`codas` on docs.rs](https://img.shields.io/docsrs/codas)](https://docs.rs/codas/)
[![`codas` is MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/withcaer/codas/blob/main/LICENSE.txt)

Markdown-defined data that serialize to and from bytes
on any platform—from web apps to robots!

## What's a Coda?

Codas document the structure of _related_ types of
**data**, each containing one or more **fields**.
Codas' data can encode to and decode from raw binary
data streams, which makes them extra-helpful for
building distributed or embedded apps that speak
different languages or rely on low-level networking.

> _Note_: For those familiar with other data
> interchange formats, codas are similar to
> [Protocol Buffers](https://github.com/protocolbuffers/protobuf)
> and [Cap'n Proto](https://capnproto.org/).

Each data type in a coda can have the following
kinds of fields:

1. Unsigned integers from `8` to `64` bits
   (`u8`, `u16`, `u32`, and `u64`).
2. Signed integers from `8` to `64` bits
   (`i8`, `i16`, `i32`, and `i64`).
3. Signed floating-point integers from `32` to `64` bits
   (`f32` and `f64`).
4. Booleans (`bool`).
5. UTF-8 encoded text (`text`).
6. Fixed-size arrays of any fixed-size type
   (`array of N u8`, `array of N f32`, and so on).
7. _Other_ user-defined data types ("nested" data)
8. Lists of any of the things listed above.

For information on how codas' data is coded to and
from binary data, refer to the [`codec`](https://docs.rs/codas/latest/codas/codec) docs.

## How do I make a Coda?

Codas are made with Markdown:

```markdown
# `Greeter` Coda
An example coda.

## `Request` Data
Data type in this coda named `Request`.

+ `message` text

## `Response` Data
Another data type in this coda named `Response`.

+ `message` text

+ `friends` list of text

   This field of a `Response` is a list
   of text, instead of a single `text`.

+ `request` Request

   This field of a `Response` is a copy of the
   `Request` that the response is for, showing
   how we can nest data types within each other.
```

This example describes a `Greeter` coda with two
kinds of data: `Request` and `Response`. Both of
these data contain a `message` text, while the
`Response` data contains a _list_ of text called
`friends` and a copy of the original `Request`:

- Every coda begins with a header (`#`) containing
the name of the coda (`Greeter`, in this example)
followed by the word `Coda`.

- Every data description begins with a header (`##`)
containing the name of the data type (`Request` or `Response`,
in this example) followed by the word `Data`.

- Each field inside of a data description is a list item,
starting with a `+` and followed by the _name_ and then
the _type_ of the field.

- Any text directly below a coda header, data header,
or field item will be parsed as Markdown documentation
for that item.

The _order_ of `Data` and their fields (`+`) matters: If
data or fields are re-arranged, the binary encoding of that
data may also change.

## How do I use a Coda?

The easiest way to get started with Codas is with Rust via
the [`codas-macros`](https://crates.io/crates/codas-macros)
crate.

### From Other Languages

Try out the live code generator on [codas.dev](https://www.codas.dev)!

## Can I evolve or extend my Codas?

Yes! Codas are designed to evolve as a system's
needs change: 

- New data types can be added to the end of a coda.
- New fields can be added to the end of a data type.
- Existing fields and data types can be renamed.

If a system receives data of a new type it doesn't
support, or containing new fields it doesn't support,
the new information will be gracefully ignored.

Conversely, if a system receives data that's _missing_
newly-added fields, the missing fields will be gracefully
populated with default values.

## Relative Performance [("Benchmarks")](https://github.com/withcaer/codas/blob/main/codas/benches/codecs.rs)

Operation | `codas` | `prost (proto3)`
--|--|--
Encode | `49ns (20M/s)` | `51ns (19.6M/s)`
Decode | `110ns (9M/s)` | `118ns (8.5M/s)`

> Comparative performance of different scenarios we've written
> benchmarks for. Exact numbers will vary between platforms.

## Related Crates

- [`codas-macros`](https://crates.io/crates/codas-macros): Macros for
  generating Rust data structures for any Coda.
- [`codas-flow`](https://crates.io/crates/codas-flow): Low-latency,
  high-throughput Bounded queues (\"data flows\") for (a)synchronous
  and event-driven systems.

## License

Copyright © 2024 - 2026 With Caer, LLC and Alicorn Systems, LLC.

Unless otherwise specified, the crates in this repository are licensed under the MIT license. Refer to [the license file](https://github.com/withcaer/codas/blob/main/LICENSE.txt) for more info.

> _Note_: Codas and their related Rust Crates were originally maintained
> by [Alicorn Systems on GitLab](https://gitlab.com/alicorn/pub/alicorn).
> On May 12th, 2025, Alicorn Systems, Inc. transferred Codas and their related
> Rust Crates to With Caer, LLC.