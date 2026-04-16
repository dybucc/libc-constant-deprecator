#import "@local/typst-template:0.40.0": *

#show: template.with(
  title: [Notes --- `libc` constant deprecator],
  authorship: (
    (
      name: "Adam Martinez",
      email: "adammartinezoussat@gmail.com",
      affiliation: "University of Life",
    ),
  ),
)

= Expanding macros

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
