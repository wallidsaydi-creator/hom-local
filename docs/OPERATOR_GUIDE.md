# HOM Local LLM Operator Guide

Date: 2026-05-18
Status: source-faithful operator guide
Audience: an LLM or agent operating through the HOM Local app/runtime

## Current Architecture Boundary

HOM Local is a native macOS shell plus one app-facing ingress server on `127.0.0.1:9101`. The Rust brain is the cognitive and security core: memory, recall, nightly dreams, reasoning, ledger, identity/session state, diagnostics, and permission gates. The ingress capability mesh owns provider discovery/chat dispatch and brokers browser, file, shell, network, skills, plugins, and MCP descriptors through permission-gated tools. External executors stay outside the brain, but their descriptors and action traces are registered back into brain memory.

The operator must not route provider, model, chat dispatch, tool execution, skill indexing, plugin, or MCP ownership back into the brain. If a capability is `offline`, `missing`, `denied`, or `degraded`, report that exact state and continue with the capabilities that remain available. Legacy runtime-bubble and route-certificate language in older reports is historical context; current readiness is the selected provider/model plus registered, permission-gated capability mesh tools.

## Purpose

This guide explains how to operate HOM Local safely and effectively from the point of view of an LLM working through the app.

It covers:
- what exists,
- what does not,
- how login/session state works,
- how save/recall/chat/provider flows work,
- what the security and permission boundaries are,
- what the agent must never assume.

This is not a marketing document. It is an operator manual.

---

## 1. Core Operating Principle

HOM Local is a **truthful local runtime**.

As an operator, you must assume:
- if the backend says a capability is unavailable, it is unavailable,
- if the backend says a provider is not routeable, you must not act as if it is,
- if a registry is empty or degraded, you must not invent rows,
- if permission is missing, you must not bypass it,
- if a signed path is required, unsigned usage is not valid.

Do not smooth over degraded or unavailable states with guessed output.

---

## 2. Main Runtime Surfaces

The native app calls the ingress HTTP service, which calls the brain over signed JSON-RPC.

### Public ingress surfaces
The source declares public endpoints including:
- `/.well-known/hom-local.json`
- `/api/health`
- `/api/ready`
- `/openapi.json`
- `/api/tools`
- `/api/auth/request`
- `/api/auth/status`
- `/api/recall`

### Main UI surfaces used by the app
The macOS client expects and uses routes such as:
- `/api/ui/session`
- `/api/ui/login`
- `/api/ui/logout`
- `/api/ui/status`
- `/api/ui/runtime-status`
- `/api/ui/chat`
- `/api/ui/recall`
- `/api/ui/recall/smart`
- `/api/ui/recall/:id`
- `/api/ui/memory/:id`
- `/api/ui/projects`
- `/api/ui/sessions`
- `/api/ui/settings`
- `/api/ui/skills`
- `/api/ui/tools`
- `/api/ui/plugins`
- `/api/ui/apps`
- `/api/ui/providers`
- `/api/ui/contract`
- `/api/ui/monitoring`
- `/api/ui/permissions`
- provider-candidate, route, credential, helper, catalog, and maintenance routes

The brain worker currently dispatches 128 methods behind these surfaces.

---

## 3. Session, Login, and Identity

### What the app does
The macOS app source (`HOMLocalService.swift`) calls:
- `GET /api/ui/session` to check session state
- `POST /api/ui/login` to login
- `POST /api/ui/logout` to logout

### What the brain exposes
The brain source declares:
- `session.get`
- `session.login`
- `session.logout`

### Operator rule
As an LLM:
- do not assume the user is already logged in,
- do not assume the daemon is configured or unlocked,
- check session/runtime status first,
- treat login/session state as backend-owned truth.

---

## 4. Runtime Status: What To Read First

Before doing meaningful work, the safest first read is:
- `/api/ui/runtime-status`

The source model includes:
- daemon status
- identity
- ledger state
- memory counts
- provider route state
- registry summaries
- permissions
- session summary
- diagnostics
- capability list

### Operator checklist
Before acting, inspect:
1. `daemon.status`
2. `ledger.valid`
3. `provider_route.routeable`
4. registry states for skills/tools/plugins/apps
5. permission profile and toggles
6. available capabilities

### Never assume
Do **not** assume:
- provider chat is ready because a provider row exists,
- a model is selectable because it appears in a generic list,
- skills/plugins exist because the UI has a section for them,
- the ledger is healthy without reading its status,
- permissions are broad enough for network, provider keys, automation, or shell.

---

## 5. Save, Recall, Open, Answer

### Memory surfaces that exist
The source declares and dispatches:
- `memory.save`
- `memory.recall`
- `memory.open`
- `memory.answer`

The app uses:
- `POST /api/ui/recall`
- `POST /api/ui/recall/smart`
- `GET /api/ui/recall/:id`
- `GET /api/ui/memory/:id`
- `POST /api/ui/memory/save`

### Operator behavior
#### To save
Use the save path only when there is something worth preserving.
A save is not a casual dump. It passes through:
- quality gate,
- security boundary,
- memory persistence,
- ledger recording.

#### To recall
Use recall when you need supporting context or prior work.
Use memory-open when you already have the exact ID and need the full record.

#### To answer
Treat `memory.answer` / chat answer flows as evidence-backed outputs, not automatic truth.
If evidence is thin, the operator should acknowledge that and avoid pretending certainty.

---

## 6. Chat and Provider Routing

### Important rule
Chat is not just "pick a provider and send."

The current architecture is route-certificate-oriented.
A provider/model path must be proven routeable before chat should be treated as available.

### What the app source expects
The macOS app calls:
- `POST /api/ui/chat`
- `GET /api/ui/providers`
- `POST /api/ui/providers/:id/test-chat`
- provider-route and candidate routes

The app model expects fields such as:
- provider id
- model id
- route certificate id
- credential state
- routeable bool
- route-certified models
- auth contract

### Operator rules
- never treat provider existence as routeability,
- never treat static provider labels as proof,
- never choose a model unless the backend says it is route-certified / available in the current route context,
- if no route exists, the correct outcome is disabled/unavailable/needs setup — not a guessed answer path.

---

## 7. Auth, Credentials, and Certification

### Auth flow in source
The brain auth service declares:
- `auth.lanes`
- `auth.request`
- `auth.status`
- `auth.complete`

### Secret handling rule
`auth.complete` explicitly rejects raw secret material such as:
- `api_key`
- `secret`
- `token`
- `access_token`
- `refresh_token`

It accepts references only:
- `credential_ref`
- `oauth_ref`

### Operator rules
- never send raw provider secrets into HOM auth completion,
- expect reference-based completion, not raw key storage,
- understand that auth lanes are provider-specific,
- unsupported OAuth/API modes should remain unavailable rather than silently falling back.

### Route certificates
As an operator, treat route certification as the gating truth for positive provider use.
No route certificate means no real provider chat path.

---

## 8. Permissions and Grant Boundaries

### Permission profiles in source
Current profiles:
- `restricted`
- `workspace`
- `full_access`

### Grant kinds in source
Current explicit grant kinds:
- `shell`
- `filesystem`
- `app_automation`
- `mcp`
- `plugin`
- `provider_key_access`
- `network`
- `cloud_drive`

### Permission fields the runtime exposes
The source permission state includes:
- `network_enabled`
- `provider_key_access`
- `automation_enabled`
- `shell_access`
- local root grants
- cloud/local filesystem grant lists

### Operator rules
Before attempting actions that rely on external or sensitive capabilities, inspect permissions.

Do not assume:
- network is enabled,
- provider key access is enabled,
- automation is enabled,
- shell access is enabled,
- local filesystem roots are granted.

If permission is missing, the right behavior is to stop and surface the reason.

---

## 9. Security Layer

The security layer is not advisory. It is an enforcement boundary.

### Source-declared security decisions
The security policy can deny for reasons including:
- canary propagation
- secret exfiltration
- memory poisoning
- recall exfiltration
- prompt relay / prompt injection
- unknown method default deny

### Operator rules
Never try to:
- persist policy overrides in memory,
- ask for bulk exfiltration of all memories,
- inject or relay prompt-injection content,
- pass secret material into normal save/answer/auth flows,
- assume unknown methods are allowed.

A denied request is a hard stop, not a hint to retry in a sneakier way.

---

## 10. Registries and What They Mean

Current registry compartments include:
- tools
- skills
- plugins
- MCP
- apps
- package/runtime catalog state

### Operator rules
- a registry section existing in the UI does not mean rows exist,
- do not invent tools/skills/plugins/apps when registry counts are zero,
- do not mix providers into plugin/app rows,
- do not treat manifests as permission grants,
- do not treat discovery as approval.

---

## 11. Nightly, Reasoning, and Monitoring

### Nightly surfaces exist
The source declares:
- `nightly.tree`
- `nightly.dry_run`
- `nightly.run`

### Reasoning surfaces exist
The source declares:
- `reasoning.artifacts`
- `reasoning.bridge.list`
- `reasoning.bridge.backfill_imports`
- `reasoning.run`

### Monitoring surface exists
The source declares:
- `monitoring.snapshot`
- `/api/ui/monitoring`

### Operator rules
- do not present nightly/reasoning/monitoring as magical hidden intelligence,
- use them as explicit backend surfaces,
- surface degraded/unavailable states truthfully,
- do not fabricate bridge links, warnings, or reasoning artifacts.

---

## 12. What Exists vs What You Must Still Verify

### Exists in source
- broad ingress route surface
- route-certificate-oriented provider architecture
- permission profiles and grant kinds
- registry compartments
- UI contract surface
- session/project/chat/runtime structures
- auth lane and ref-only credential model
- security deny rules

### Must still be treated as runtime-verified, not merely source-declared
- which routes are live in the installed build,
- which registries are populated,
- which providers are routeable,
- whether chat is positively usable now,
- current ledger validity,
- current permission state,
- current session/project state,
- current maintenance/import/export environment.

Source declaration is not the same as live proof.

---

## 13. Recommended Operating Sequence For An LLM

When entering HOM Local through the app/runtime, do this in order:

1. Check session/login state.
2. Read `/api/ui/runtime-status`.
3. Inspect capability and degraded-state information.
4. Inspect permissions if your task may need network, provider keys, automation, shell, or filesystem.
5. Inspect providers only if your task actually needs chat/provider work.
6. Use recall/open before answering from memory.
7. Save only information worth preserving.
8. Never bypass auth, permission, route, or security boundaries.
9. Treat missing/unavailable as a real answer.
10. Prefer backend truth over UI assumption.

---

## 14. Short Consumer-Facing Interpretation

If this same guide has to be summarized for a user, the plain-language version is:

- check what is connected,
- check what is available,
- check what needs setup,
- use recall before making claims,
- save only useful context,
- respect permissions,
- do not force actions the system says are unavailable.

---

## Non-Claim Boundary

This guide does not claim that every source-declared feature is fully verified live.
It defines how an LLM should behave around HOM Local based on the current source architecture and its explicit trust boundaries.
