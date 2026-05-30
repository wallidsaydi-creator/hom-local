# Contributing

## Overview

HOM Local welcomes contributions. This guide covers development setup, coding standards, and the contribution process.

## Development setup

### Prerequisites

- Rust 1.94+ (edition 2024)
- Cargo
- SQLite (bundled via rusqlite)
- Just (optional, for task running)

### Setup

```bash
# Clone the repository
git clone https://github.com/hom-local/hom-local.git
cd hom-local

# Install dependencies
cargo build

# Run tests
cargo test

# Start development server
cargo run --bin hom-brain
```

## Code structure

### Workspace layout

```
hom-local/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── hom-brain/          # Core daemon
│   ├── hom-shared/         # Shared types
│   └── hom-ingress/        # HTTP layer
├── docs/                   # Documentation
├── examples/               # Usage examples
└── tests/                  # Integration tests
```

### Coding standards

- **Rust edition 2024**: Use latest features
- **Clippy**: No warnings allowed
- **rustfmt**: Use default formatting
- **Documentation**: All public items must be documented
- **Tests**: All new features must include tests

## Contribution process

### 1. Fork and clone

```bash
git clone https://github.com/YOUR_USERNAME/hom-local.git
cd hom-local
git remote add upstream https://github.com/hom-local/hom-local.git
```

### 2. Create branch

```bash
git checkout -b feature/my-feature
```

### 3. Make changes

- Write code following coding standards
- Add tests for new functionality
- Update documentation as needed

### 4. Run checks

```bash
# Format code
cargo fmt

# Run clippy
cargo clippy -- -D warnings

# Run tests
cargo test

# Build release
cargo build --release
```

### 5. Commit

```bash
git add .
git commit -m "feat: add new feature

- Description of changes
- Impact on existing functionality
- Tests added"
```

### 6. Push and create PR

```bash
git push origin feature/my-feature
```

Create a pull request with:
- Clear title and description
- Link to related issues
- Test results
- Documentation updates

## Development workflow

### Feature development

1. Create issue for discussion
2. Fork and create branch
3. Implement with tests
4. Update documentation
5. Submit PR for review
6. Address feedback
7. Merge

### Bug fixes

1. Create issue with reproduction steps
2. Fork and create branch
3. Fix bug with regression test
4. Submit PR with before/after comparison
5. Merge

## Testing

### Unit tests

```bash
# Run unit tests
cargo test --lib

# Run with coverage
cargo tarpaulin --out Html
```

### Integration tests

```bash
# Run integration tests
cargo test --test brain_e2e
cargo test --test ingress_e2e
```

### Manual testing

1. Start brain daemon: `cargo run --bin hom-brain`
2. Start ingress: `cargo run --bin hom-ingress`
3. Test API endpoints
4. Verify logs and errors

## Documentation

### Code documentation

```rust
/// Save a memory to the brain.
///
/// # Arguments
///
/// * `key` - Unique identifier for the memory
/// * `value` - Memory content
/// * `memory_type` - Type of memory (declarative, procedural, etc.)
///
/// # Returns
///
/// Returns the memory ID on success.
///
/// # Errors
///
/// Returns error if the save operation fails.
pub fn save_memory(key: &str, value: &str, memory_type: &str) -> Result<String, Error> {
    // ...
}
```

### Architecture documentation

Update `docs/architecture.md` when:
- Adding new crates
- Changing data flow
- Modifying IPC protocol
- Updating security model

## Code review

### Review checklist

- [ ] Code compiles without warnings
- [ ] Clippy passes without warnings
- [ ] All tests pass
- [ ] Documentation is updated
- [ ] Security implications considered
- [ ] Performance impact assessed
- [ ] Breaking changes documented

### Review process

1. PR submitted
2. Automated checks run
3. Reviewer assigned
4. Feedback provided
5. Changes requested
6. Approval granted
7. Merge

## Release process

### Versioning

HOM Local follows semantic versioning:
- **Major**: Breaking changes
- **Minor**: New features
- **Patch**: Bug fixes

### Release steps

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md`
3. Create release tag
4. Build release artifacts
5. Publish to crates.io
6. Create GitHub release

## Community

### Communication

- **GitHub Issues**: Bug reports and feature requests
- **GitHub Discussions**: Questions and ideas
- **Pull Requests**: Code contributions

### Code of Conduct

- Be respectful and inclusive
- Focus on constructive feedback
- Help others learn and grow
- Maintain professional standards

## License

By contributing, you agree that your contributions will be licensed under the Apache License, Version 2.0.

Do not include code, data, models, prompts, memory exports, screenshots, private documents, or third-party content unless you have the right to contribute them under Apache-2.0-compatible terms.

HOM Oracle is not part of this repository. Do not submit Oracle-specific code, private packs, private service definitions, or private orchestration logic.
