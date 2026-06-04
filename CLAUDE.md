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
- Use `feat:` for new features, `fix:` for bug fixes, `chore:` for maintenance.
- Breaking changes: add `!` after the type/scope, e.g. `feat!: remove deprecated API`.
- The release train auto-detects bump level from PR titles; incorrect types will either miss releases (no bump) or trigger wrong bump levels.

### Examples

```
feat: add actr_id to telemetry spans
fix(hyper): prevent duplicate disconnect firing
chore(deps): bump tokio to 1.45
feat!: remove deprecated config format
ci: add PR title validation
```
