# AGC assembly graphs

codingest parses yaYUL Apollo Guidance Computer source (`.agc`) without running
the assembler. Identifiers are qualified by the top-level program directory,
so `Comanche055.START` and `Luminary099.START` are independent graph symbols.

## Nodes

Executable labels remain `Function` nodes so ordinary cross-language queries
continue to work. They carry `symbol_kind = "agc_label"`; labels reached by a
resolved returning call also carry the conservative `role_hint = "routine"`.
Branch-only, computed-dispatch, externally entered, and fall-through labels are
not guessed to be routines.

Data definitions are `Constant` nodes. Their `kind` preserves the yaYUL
pseudo-op, including `agc_equals`, `agc_equals_alias`, `agc_erase`, numeric
literals, address words, and verb/noun codes. ERASE symbols additionally carry
`is_mutable = true` and `storage = "erasable"`.

## Relationships

| Relationship | Meaning |
|---|---|
| `CALLS` | Returning `TC`/interpretive CALL, plus resolved BANKCALL/IBNKCALL |
| `JUMPS_TO` | Non-returning `TCF`/interpretive GOTO, plus resolved POSTJUMP |
| `BRANCHES_TO` | Conditional BZF/BZMF transfer |
| `REFERENCES` | Exact program-local access from an executable label to data |
| `ALIAS_OF` | Resolved symbolic EQUALS or `=` definition |
| `POINTS_TO` | Resolved ADRES/CADR/ECADR/GENADR/BBCON definition |

Control edges preserve their source lines and count. `raw_targets`, `offsets`,
`via`, and `address_lines` retain source spelling, signed offsets, inter-bank
mechanisms, and consumed CADR/FCADR lines when present. REFERENCES retains its
legacy first `line` and adds `reference_lines`, `reference_count`, `opcodes`,
`accesses`, `has_read`, `has_write`, and `has_address`.

Register-indirect and relative-only transfers are retained internally as
unresolved sites but do not fabricate graph edges. In particular, `TC Q` is a
return idiom; BANKJUMP and SWCALL receive their destination through register A;
and `CCS operand` accesses data rather than branching to that operand.

## Queries

Find returning callers of a routine:

```cypher
MATCH (caller:Function)-[r:CALLS]->(target:Function)
WHERE target.qualified_name = 'Comanche055.IMUSTALL'
RETURN caller.qualified_name, r.call_lines, r.via
ORDER BY caller.qualified_name
```

Keep jumps and conditional branches distinct:

```cypher
MATCH (source:Function)-[r:JUMPS_TO|BRANCHES_TO]->(target:Function)
WHERE target.qualified_name = 'Luminary099.ENDOFJOB'
RETURN type(r), source.qualified_name, r.transfer_lines
```

Find writes to erasable storage:

```cypher
MATCH (source:Function)-[r:REFERENCES]->(data:Constant)
WHERE r.has_write = true AND data.storage = 'erasable'
RETURN data.qualified_name, source.qualified_name, r.opcodes, r.reference_lines
ORDER BY data.qualified_name
```

Traverse aliases and address constants without discarding their source names:

```cypher
MATCH (symbol:Constant)-[r:ALIAS_OF|POINTS_TO]->(target)
RETURN symbol.qualified_name, type(r), target.qualified_name, r.raw_target
```

## Deliberate limits

The graph does not claim a static bank for every symbol. BANK changes are
implicit and EBANK can be dynamic, so a bank model requires assembler-grade
state tracking. Instruction-level relative skip control flow (including CCS)
is likewise outside this label-level graph. Missing edges for those cases mean
“not statically established,” not “no runtime transfer.”
