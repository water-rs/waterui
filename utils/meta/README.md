# waterui-meta

The metadata-directory record format every `waterui_meta_*` artifact channel
shares — a `static` parked in a dedicated linker section (`.wmeta`;
`__DATA,__wmeta` on Apple targets) whose bytes are a self-describing
`name\0payload\0` record.

The section exists because a symbol table cannot be trusted to survive
linking: a linked PE image keeps no COFF symbol table at all, so a static
findable only by symbol name is unreachable there — the record's own name
field is what identifies it, in objects, rlibs, and linked images alike.

Zero dependencies, `no_std` + `alloc`.
