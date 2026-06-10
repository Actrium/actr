## Conventional Commits

All commit messages and PR titles MUST follow the Conventional Commits format:

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

The `type` prefix is mandatory, separated from the description by a colon and space.

### Valid types and corresponding semver bumps

| type | meaning | semver bump |
|------|---------|-------------|
| `fix:` | bug fix | PATCH (0.1.0 → 0.1.1) |
| `feat:` | new feature | MINOR (0.1.0 → 0.2.0) |
| `feat!:` or `BREAKING CHANGE` footer | breaking change | MAJOR (0.1.0 → 1.0.0) |
| `chore:`, `docs:`, `style:`, `refactor:`, `test:`, `ci:`, `perf:`, `build:`, `revert:` | no API impact | no bump |

### When creating a PR

- The PR title IS the squash merge commit message.
- Breaking changes: add `!` after the type/scope, e.g. `feat!: remove deprecated API`.


### Commit type selection rules (priority order)

**For 0.x early development:** Prefer `fix:` over `feat:` unless the change exposes a clearly new API or feature to end users. Most internal improvements, FFI helpers, and incremental additions qualify as `fix:`.

1. **`fix:`** — Use for bug fixes, small enhancements, internal helpers, FFI bindings, incremental refinements, and non-user-facing improvements. This is the default for 0.x development.
2. **`feat:`** — Use ONLY when the change is a genuinely new end-user-facing capability or API surface. Do NOT use for internal helpers, small refinements, or incremental polish.
3. **`feat!:` or `BREAKING CHANGE` footer** — Public API removal, signature changes that break callers, or incompatible protocol changes.
4. **`refactor:`** — Pure internal restructuring with zero behavioral change and no new capability.
5. **`chore:`, `ci:`, `build:`, `docs:`, `test:`** — Maintenance, CI, dependencies, documentation, tests.

- The release train auto-detects bump level from PR titles; incorrect types will either miss releases (no bump) or trigger wrong bump levels.

### Examples

```
feat: add actr_id to telemetry spans
fix(hyper): prevent duplicate disconnect firing
chore(deps): bump tokio to 1.45
feat!: remove deprecated config format
ci: add PR title validation
```
