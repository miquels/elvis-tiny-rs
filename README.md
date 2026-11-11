# elvis-tiny-rs

Elvis 1.4 was packaged for Debian as elvis-tiny. It was compiled with a minimum feature-set
so that the executable was very small and could be used for a small root filesystem or
rescue disk, while still having a "real" vi editor.

However this code was old and typical of C projects from the 90s: static buffers and pointer
arithmetic all over the place and full with buffer overflows.

Rather than fixing the old C codebase, the code was ported by Claude Opus to Rust. It only
has the basic features that elvis 1.4 as elvis-tiny was compiled with, and only has been
tested on macOS and Linux.

The size of the final executable is comparable to the C version.

The porting instructions can be found in IMPLEMENTATION.md. The elvis-1.4 source code
including the patches that were carried in the debian version can be found in the
elvis-tiny-c subdirectory.

# Building on Debian

When building on Debian with the system cargo and rust, you need to install
the following packages as well:

- cargo
- rust-src
- librust-libc-dev

For development you might want to add:

- rustfmt
- rust-clippy
- unifdef
- time

# C version license.

The original license can be found in elvis-tiny-c/README and is as follows:

```
  Author: Steve Kirkendall

  Elvis is freely redistributable, in either source form or executable form.
  There are no restrictions on how you may use it.
```

# Rust code license.

```
  Short version for non-lawyers:

  This project is dual-licensed under Apache 2.0 and MIT terms.

  It is Copyright (c) The Elvis-tiny-rs Contributors.

  Longer version:

  Copyrights in this project are retained by their contributors. No
  copyright assignment is required to contribute to this project.

  Some files might include explicit copyright notices and/or license notices.
  For full authorship information, see the version control history.

  Except as otherwise noted, this project is licensed under the Apache License, Version
  2.0 <LICENSE-APACHE> or <http://www.apache.org/licenses/LICENSE-2.0> or the MIT
  license <LICENSE-MIT> or <http://opensource.org/licenses/MIT>, at your option.
```
