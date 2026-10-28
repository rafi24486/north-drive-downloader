# North Drive Downloader — V1.0 commercial release plan

## Current development state

Working baseline: v0.1.2 DMG packaged on GitHub Actions. V1.0-development currently introduces UX and authentication integration changes in a **beta**, not a production release.

## Must-pass launch gates

- [ ] macOS: native titlebar drag, native traffic lights, resizing 760px–wide desktop; QA on Apple Silicon + Intel.
- [ ] Selection behavior: non-editable UI not selectable; URL, destination, search and settings remain editable/copyable.
- [ ] Onboarding: publisher-managed Google OAuth Desktop client ID; users never asked to create a Google Cloud project or API key.
- [ ] Google Cloud OAuth consent app verified for **drive.readonly** restricted scope; set up policy, verified domain, support and data-use disclosures; satisfy any required security review.
- [ ] Google Drive: verify with public and restricted sample folders, link resource keys, Shared Drives, empty folders, Google Docs exports, and files with download disabled.
- [ ] Transfer safety: test 10,000 files, 100 GB+ files, sleep/wake, outage, retry/backoff, partial resume, duplicate-name collisions, full-disk scenarios and cancellation.
- [ ] Persistence: queue survives restart; log failed files; accurate progress/ETA; clear explanations on errors and quota.
- [ ] Seller readiness: Developer ID sign + Apple notarization, update mechanism with signed updates, crash/diagnostic opt-in, Privacy Policy, ToS, support and refund process.
- [ ] Licensing + payments: payment provider and secure license verification; no secret embedded in desktop app; offline grace policy.
- [ ] Accessibility: keyboard navigation, VoiceOver labels, focus states, 200% UI scaling and reduced motion.

## Authentication / commercial risk

- Current beta optionally reads `NORTH_GOOGLE_CLIENT_ID` at compile time. Store publisher Desktop OAuth Client ID as GitHub repository variable; **it is not a secret**. Do not store a client secret or refresh token in GitHub source.
- Google Drive OAuth `drive.readonly` is a **restricted scope**: commercial public distribution requires Google verification. Do not sell until approved. If storing restricted-scope user data on servers, further assessment may apply.
- `drive.file` has a lighter review path but is **not a drop-in replacement** for reading arbitrary shared folder links; evaluate Picker-driven consent separately.
- Direct anonymous/public folder downloads still need API access: evaluate use of signed-in OAuth for public folders or a quota-managed service without shipping unrestricted API keys.

## Milestones

**M1 Foundations (this beta):** draggable window, controlled text selection, responsive shell, first-party OAuth injection option, cleaner settings.

**M2 Real Google flow:** one-click login, access checks, shared folder handling, account switch and helpful auth errors; Google verification submission.

**M3 Transfer reliability:** durable jobs, integrity reports, background/resume, retry thresholds, controlled large-file stress tests.

**M4 Commercial QA:** signing/notarization, billing/license, licensing terms, support, accessibility and production release candidates.

**Release rule:** The version number 1.0 is reserved for when every Must-pass gate above is verified on actual Macs.

## M2 scope change — frictionless buyer sign-in

- [x] Removed buyer-facing API key and OAuth client ID configuration fields.
- [x] Switched folder scanning and download plans to authorized user OAuth access; no public API key required for signed-in users.
- [x] Added OpenID Connect `openid email profile` scopes and Google UserInfo profile retrieval.
- [x] Wired name, email, and optional profile photo into sidebar, topbar, account selector, and account detail UI.
- [ ] **Publisher action:** create and verify Google OAuth Desktop application, then configure `NORTH_GOOGLE_CLIENT_ID` as a GitHub repository variable. Without this, sign-in is intentionally unavailable in the beta; the client ID is *not* a user-facing setting.
- [ ] Test public-link resource keys and private client folders under the signed-in identity on a real Mac; Google restricted-scope verification still required for selling.
- [ ] Account persistence, token expiry, and offline profile fallback QA.

## Beta fix: 0.2.0 Google login and Drive access diagnosis
- [x] Detect stale/expired Google Keychain session on startup; show **Continue with Google** instead of dead-ending as Connected.
- [x] Check the Google Drive API immediately after OAuth; show explicit instructions when project API is disabled.
- [x] Persistent error text in Accounts; manual Check Google connection action.
- [x] Human-readable OAuth failure categories and simple regression tests.
- [ ] Confirm Google Cloud OAuth client is **Desktop app** and the consent screen includes the tester's Gmail.
- [ ] Verify the Google Drive API is enabled in the *same project* that owns the OAuth client ID.
- [ ] Run a real Google sign-in + Drive folder download on macOS. CI proves only compile/packaging, not API availability or authorization.
