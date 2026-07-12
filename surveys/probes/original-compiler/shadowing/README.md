# Shadowing Probes

Focused original-compiler probes for Action! name shadowing behavior.

These probes live in a separate directory so source-level semantic questions do
not get mixed into the broad byte-for-byte compatibility sweep.

The current question is whether declarations in an inner scope can shadow names
from an outer scope or from the resident/predefined environment.

## Probes

| Source | Host name | Output | Question |
| --- | --- | --- | --- |
| `local_var_shadow.act` | `SHADLOC.ACT` | `SHADLOC.COM` | Does a routine-local variable shadow a global variable with the same name? |
| `param_shadow.act` | `SHADPAR.ACT` | `SHADPAR.COM` | Does a routine parameter shadow a global variable with the same name? |
| `global_builtin_shadow.act` | `SHADGBI.ACT` | `SHADGBI.COM` | Can a user global shadow a predefined/resident name such as `color`? |
| `local_builtin_shadow.act` | `SHADLBI.ACT` | `SHADLBI.COM` | Can a routine local shadow a predefined/resident name such as `color`? |
| `duplicate_same_scope.act` | `SHADDUP.ACT` | none expected | Does the original reject duplicate variables in the same scope? |

## Running

Use the helper script:

```sh
surveys/probes/original-compiler/shadowing/run-vm.sh all
```

Outputs are written under `outputs/vm/`. Expected-failure probes are considered
successful when the original compiler does not write an object file.

## VM Results

Captured with `run-vm.sh all`.

| Probe | Original result | Evidence |
| --- | --- | --- |
| `local_var_shadow.act` | Compiles, `31` bytes | `x=$11` stores to local storage at `$3002`; `result=x` loads from `$3002` and stores global `result` at `$3001`. |
| `param_shadow.act` | Compiles, `29` bytes | Parameter `x` is saved at `$3002`; `result=x` loads from `$3002`, not global `x` at `$3000`. |
| `global_builtin_shadow.act` | Compiles, `30` bytes | User global `color` lives at `$3000`; stores/loads use `$3000`, not predefined `color` at `$02FD`. |
| `local_builtin_shadow.act` | Compiles, `30` bytes | Routine-local `color` lives at `$3001`; stores/loads use `$3001`, not predefined `color` at `$02FD`. |
| `duplicate_same_scope.act` | Compile error, no object file | Original reports an error at the second `BYTE x`; the helper treats this as expected failure. |

Current conclusion: Action! allows inner declarations to shadow outer symbols,
including predefined names, but rejects duplicates in the same scope.
