= Implementation notes

Expanding macros prior to parsing constants poses a few challenges. Firstly, regular expansion would
force some further form of parsing prior to reading in contents all at once, as right now I know
only of options to perform macro expansion on the entire crate, but files are parsed one at a time.
We could avoid that by changing the parsing process altogether from a directory walking-based
approach to performing a single command call to `rustc` for macro expansion, gathering afterwards
all expanded code into a single in-memory buffer. The issue would then be finding a way to trace
back all modifications done on the abstract, in-memory objects to a specific source file.

Thinking it further, deprecation of constants declared purely within a macro's body, and not within
a module that is conditionally included through a macro would be non-trivial. This would be a big
stopper to macro expansion, and thus to performing complete constant scanning and modification from
and to source. This could be solved by implementing some form of manual parsing on the inner tokens
of the macros upon `syn` stumbling on them. I believe all macros expect "reasonable" syntax in their
matchers, so maybe this is a feasible option (if not the only option.)

To recap, the goal of the library is to allow adding `#[deprecated]` attributes to constants across
the codebase, which implies two requirements.
1. The constants should be sourced from files. This is easily done with `syn` so long as the symbols
  are readily available and not part of macro bodies.

2. The constants should be traced back to files. This is also easy when parsing with `syn` because
  parsing the file anew and deprecating symbols with the same identifier is simple as long as they
  are not part of macro bodies.

In both contexts, the issue always appears when taking into consideration constant parsing within
macro bodies. Then again, solving this requires addressing two different issues corresponding to
each of the above concerns.
1. To source constants from a file, the file could be macro expanded through `rustc`'s
  `-Zunpretty=expanded` option, and the output passed to `syn`. This would remove traceability
  information from the file where the constants were sourced from.

2. To effect changes onto disk, a custom parser would have to be implemented for tokens within macro
  bodies. Whenever `syn` stumbled upon a macro, the parser would come into play, and produce
  in-memory representations equivalent to those output by `syn`.

The latter approach comes closer to providing a decent solution. This is due to the fact macro
expansion relies on `cfg` directives in the target platform that is running `rustc` for expansion.
An incomplete set of constants would be parsed depending on the environment of the user running the
deprecator program. A solution could go through iterating through all allowed (as per `check-cfg`)
directives, and expand the `libc` codebase with each of them. This would still leave open the
question of traceability of constants back to files, making the roundtrip from in-memory
representation to files almost impossible to perform accurately.

Another issue is that the traceability of constants is ruined if taking into consideration the
possibility of multiple, conditional definitions within a single file. This may be solved by adding
information to both the in-memory representation of the constant, as well as the changes persisted
onto disk. This information should consist of both the line number and the column number of the
constant. Ideally, only the line number should be included, but the changes intended by this tool
should persist in the codebase. This implies that formatting should be upkept. This is simple
outside of macro bodies, but not so within them. Because the extent of the modifications only
amounts to adding a `#[deprecated]` attribute annotation on top of module items, storing the line
number and the column number should be enough to achieve formatting stability.

From the above discussions, we can conclude that the best strategy so far would be to implement a
custom macro parser and increase the amount of tracing metadata saved per-constant. This should
provide both a possibility to parse constants defined within macro bodies in the scanning stage, as
well as to perform changes on the codebase without fear of confusing some equally named constant
within a single file or between multiple files.

Another issue to consider would be managing the use of the deprecator program by different
individuals across different revisions of the codebase. This is not going to be discussed further
until the above parser is implemented.

The macro parser requires only implementing support for the `cfg_if` macro. Currently, the `libc`
codebase only depends on a crate internal to the `rustc` compiler, so all of its macros are defined
under `crate::macros`. This makes it straightforward to check which of the macros could be used with
constants within them. Inspection of that module leads me to believe the only macro that produces
the types of constants that we are likely to modify is the `cfg_if` macro.

The macro parser has been fully implemented and is only pending testing. The last part of the
library that remains to be implemented from scratch (thus barring the refactor of the entrypoint
routines) is the Cargo integration.

This should likely consist of a single file embedded inside the `Cargo.toml` manifest to check for
the path of the file actually containing the on-disk representation of the constants. One possible
alternative is to reimplement all parsing functionality in terms of a TOML parser and directly embed
in the corresponding subtable of the `metadata` subtable of `libc` the serialized contents. This may
or may not be a good idea, considering such information would likely have to be kept upstream for
all maintainers to peruse and possibly modify whenever some contributor decided to continue the
constant deprecation process.

The most feasible approach may go through a combination of both the current implementation and of
the above proposed idea. This should likely go through making the file path of the configuration
file available through the `metadata` table of the root manifest file in `libc` (the crate as it
doesn't use workspaces,) and later accessing such a file and parsing it as TOML. This would also
allow checking off one other `TODO` in the codebase, as the current facilities for
serialization/deserialization are not quite Rusty.

One third approach may go through using a single file and not writing to the manfiest file at all.
This would make the `.toml` file be used as that of the `askama` or the `cbindgen` projects. There
would be no further roadblocks compared with the above proposals, and it would make it considerably
more hygienic to maintain, as no changes to existing assets in the codebase beyond those effected by
the tool itself would be made.

The implementation details of this last proposal would include rewriting the `ConstContainer`
methods for serialization and deserialization, as well as including another function entry point for
loading into memory the contents of some prior serialized constants. This should routine likely just
produce a `ConstContainer`, take no arguments and look straight for the relevant project root
directory path.

One last thing that remains to be discussed is the second use of the library utilities. We speak
here of second to refer to some use following a prior deprecation, that has likely been effected to
the actual codebase, and after making use of some form of source control, is likely in conflict
between the marshalled output and the current state of the codebase. This should be fairly simple to
solve, as version control gets most things out of the way for us. In the general case where some
library user has an unsynchronized state between the existing configuration file and the current
codebase, the library should simply perform a read of the state of the codebase, modifying the
in-memory deserialized representation. Even though we have so far spoken of multiple entry points
into the crate, the only real one that would be exposed to the final user would be this one. File
and constant parsing is not something that will be exposed to the user in the final binary.

The data format, considering it is now to be implemented as a TOML file, is going to require some
changes. The prior format used a terse specification that captured only the required information for
the in-memory representation of constants. In TOML, it may be preferable to parse each constant as
an individual table each. Following this approach, an example serialized file containing two
constants, `SOME_CONSTANT` and `SOME_OTHER_CONSTANT` would look like:
```toml
[SOME_CONSTANT]
deprecated = false
source = "/Users/dybucc/rust-dev/libc/src/hermit.rs"
line = 1
column = 2

[SOME_OTHER_CONSTANT]
deprecated = true
source = "/Users/dybucc/rust-dev/libc/src/hermit.rs"
line = 10
column = 50
```

Even though there's no type checking at the TOML table level, this should provide the application
with enough information to fetch into memory some existing deprecation state, and either (1) update
it with a new pass over the `libc` codebase, or (2) modify entries in it by toggling the deprecation
notice.

In the former case, updating would likely require complete reparsing, as both the in-memory
representation and the marshalled output should keep track of all constants currently declared in
all target environments. This, though, implies that there is no real need for a file to cache
on-disk a record for each constant that has been selected for deprecation but perhaps isn't marked
with the corresponding attribute just yet. If scanning of the codebase is always bound to happen to
avoid inconsistencies between the file containing the records and the current state of the codebase,
the only purpose of keeping such records becomes to allow marking deprecation without effecting
those changes in the same "depreaction session." This, in and of itself, makes no sense as end users
of the `fzf`-like picker are likely to require immediate deprecation of the constants that they have
selected. There is no need to keep some constants as "marked for deprecation" because there does not
exist a relation of dependence between constants in this particular context.

The conclusion is that all functionality related to having constants saved on disk is useless and
should be removed. Efforts should focus on having all I/O bound functionality be asynchronous.

The heavy work of discovering/cloning the repo in the `scan_files()` entry point to the crate is now
async. The directory traversal that was previously implemented in terms of the facilitites provided
by the `walkdir` crate should likely be refactored into using manual directory traversal, as that
library does not seem to offer an async alternative. The routine attempts to access the source
directory of the `libc` crate, but the current implementation would also parse all other crates in
the repo as it is currently not using Cargo workspaces but rather crates inside the same path as the
manifest file for the main `libc` crate. This is incorrect, and likely means the path ought be
hardcoded, which should also drop the dependency on `cargo_metadata`. Beyond that, implementing
recursive directory traversal should be fairly straightforward with `tokio`'s stream adaptors. That
may not be necessary as the `tokio` docs explictly mention that the operation is done on a separate
thread to run the same function as that found in `std`. This means there's no real asynchronicity
here beyond the separate execution the extra thread provides. That likely means we can still rely on
`walkdir`, but drop `cargo_metadata`.

Now all I/O-bound functions participating in `scan_files()` are async. The only thing that remains
is to increase the level of parallelism, as parsing depends on the paths that are fetched right
before, but it iterates sequentially through those paths, so they can be made to be transmitted
through a channel between the routine that fetches the paths and the routine that reads the contents
of the files given those paths. This should void the need for gathering a collection of paths from
the fetching routine, as they would be streamed to the parsing routine as soon as they were made
available.

The entirety of `scan_files()` is now async. The only other part of the library that performs
I/O-bound operations is the method on `ConstContainer` that persists the changes to disk. Making
that async and actually exploiting parallelism in `tokio` tasks is going to require having pointers
to the underlying types for each of the constant's fields. This has been completed and should work
lest the bound on non-`Send` types is meant to infer further meaning than that found when handling
raw threads.

The binary should allow specifying the path to the `libc` crate through a CLI argument, but beyond
that there should not be any need to set up further pre-TUI logic. The TUI should consist of a
single prompt at the lower end of the screen, followed by a list of constants expanding all the way
until the very top of the screen. This likely means having to use `ratatui` and alternate screen
mode, which in and of itself means a lot of UI work. Initially, and to test out the library, it'd be
best if the binary didn't launch any UI, but rather used `crossterm` without alternate screen mode
enabled but with raw mode enabled. Then, the prompt would be displayed right after the line to
launch the command in the user's shell, and the list of constants that got filtered would appear
right below the prompt. The list would be refreshed every 1 second after the last keypress, which
should yield relatively simple rendering logic at the cost of snapyness.
