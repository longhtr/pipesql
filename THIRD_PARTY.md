# Third-party attribution

PipeSQL uses the following third-party material. Include the applicable terms
with source and binary distributions.

| Material | Provenance and terms |
| --- | --- |
| Vendored libc | Pinned by Cargo.lock; [MIT](vendor/libc/LICENSE-MIT) and [Apache-2.0](vendor/libc/LICENSE-APACHE). |
| GoogleSQL query fixtures | Pinned source and unchanged headers are described in [fixture provenance](tests/fixtures/upstream/README.md); [Apache-2.0 license](tests/fixtures/LICENSE.google-zetasql.txt). |
| Darwin canonical-name traversal | Rust adaptation in `filesystem/src/syscall/path.rs` of Apple/FreeBSD `realpath.c`; terms below. |

## Canonical-name traversal

Source: `apple-oss-distributions/Libc`, revision
`3993268b10453da1651f3fbf54dee16fad6e0dc6`, `stdlib/FreeBSD/realpath.c`.
Source SHA-256: `9c0c18bafece4fc0a877374502389f701cf44f292489620f8feec0313eb79052`.
The adaptation uses call-local state, checked storage, and explicit work admission.
This provenance does not identify the installed operating system's implementation.

Copyright (c) 2003 Constantin S. Svintsoff <kostik@iclub.nsu.ru>

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.
3. The names of the authors may not be used to endorse or promote
   products derived from this software without specific prior written
   permission.

THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHOR OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
SUCH DAMAGE.
