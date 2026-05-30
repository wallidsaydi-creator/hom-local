# Security

## Overview

HOM Local implements defense-in-depth security with multiple layers of protection for local-first operation.

## Authentication

### Ed25519 envelope authentication

All IPC communication uses Ed25519 signed envelopes:

1. Client generates keypair
2. Requests are signed with private key
3. Brain verifies signature with public key
4. Nonces prevent replay attacks
5. Timestamps ensure request freshness

### Request signing

```json
{
  "client_id": "my-app",
  "client_pub": "base64-public-key",
  "timestamp": 1700000000,
  "nonce": "unique-nonce",
  "scope": "memory:recall",
  "body_hash": "sha256-of-body",
  "signature": "ed25519-signature"
}
```

## Security gates

### Input quarantine

Malicious inputs are quarantined before processing:

- **Data exfiltration detection**: Blocks attempts to dump all memories
- **Prompt injection**: Detects and blocks injection attempts
- **Scope enforcement**: Ensures requests stay within authorized scope

### Permission enforcement

Every tool invocation goes through permission checks:

1. **Descriptor check**: Tool must be registered
2. **Domain authorization**: Tool domain must be allowed
3. **Profile enforcement**: Permission profile must permit the operation
4. **Gate task creation**: Audit trail for every invocation

## Quality gates

### Four-wall assessment

Every memory passes through quality gates:

1. **Form**: Structural correctness
2. **Filter**: Relevance and deduplication
3. **Substance**: Evidence strength
4. **Factuality**: Atomic precision

### Evidence atoms

Factual precision uses EvidenceAtom/FActScore-style evaluation:
- Claims are decomposed into atomic facts
- Each fact is scored for support
- Unsupported claims penalize confidence

## Data protection

### Local storage

- All data stored locally in SQLite
- No cloud sync by default
- WAL mode for concurrent access
- Hash chain integrity verification

### Credential management

- API keys stored in OS keychain
- Credentials never returned in API responses
- OAuth tokens managed through secure flows
- Secret material marked as non-returnable

### Backup integrity

- Backups include ledger verification
- Hash chain validated on restore
- Import sources tracked for provenance

## Audit trail

### Ledger events

Every mutation is recorded in the append-only ledger:
- Event type and timestamp
- Hash chain linking
- Payload for replay
- Verification status

### Tool event recording

Every tool invocation creates:
- Tool event record
- Bridge event linking to memory
- Benchmark result for evaluation
- Action trace for debugging

## Network security

### Local-first operation

- Brain IPC uses Unix domain sockets
- No network exposure by default
- HTTP ingress binds to localhost
- TLS optional for external access

### Rate limiting

- Per-client rate limits
- Configurable windows and thresholds
- Automatic quarantine on abuse
