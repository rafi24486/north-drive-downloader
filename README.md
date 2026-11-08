# North Drive Downloader v0.2.0

macOS Google Drive folder downloader. Dark / pink original North UI, now with recoverable transfers and a minimal Settings-only credential flow.

## What's changed

- **Resumable large downloads:** `.northpart` data survives temporary Google Drive errors, app restarts, and cancellation. An interrupted active job is restored and automatically continued on relaunch; a cancelled or failed job appears with a **Resume** action.
- **Google API error handling:** Parses Google's structured 403 reasons, retries temporary quota / rate-limit and server failures with backoff, refreshes expired OAuth tokens, and rejects permanent permission failures without pretending they are recoverable.
- **Part-file self-repair:** Automatically replaces incompatible partial metadata, restarts when the server ignores a byte range, checks downloaded size, and retries a file after a checksum mismatch. Existing verified footage is preserved.
- **UI reliability:** Less frequent transfer redraws, responsive layout, wider window-resizing range, readable file error messages.
- **Minimal interface:** No Accounts tab, no lower-left promotional cards, no tip panel or "Powered by" footer. The credit links to [Muhaimenun Amin Rafi](https://www.instagram.com/muhaimenun.rafi/).
- **Bring your own Google API credentials:** The repository bundles **no API key, OAuth Client ID, or Google account credentials**. Every downloader configures their own values in Settings.

## Set up Google Drive access

1. In [Google Cloud Console](https://console.cloud.google.com/), create your own project and enable **Google Drive API**.
2. For a public shared folder, create an API key and enter it into **Settings → Google Drive API Key**. Scope/restrict the key appropriately.
3. For private/restricted folders, create an **OAuth Desktop Client ID** in the same project and enter it under **Settings → OAuth Desktop Client ID**. Select **Authorize private folders** in Settings; the authorization opens your browser and stores a refresh token in macOS Keychain.

**Important:** An OAuth Client ID alone does not grant access to private Drive content. Browser-based Google permission is required for the user who owns or can access the shared folder. Whether a particular Google Desktop Client accepts a client-ID-only PKCE token exchange depends on Google's OAuth service; private-folder OAuth has not been live-tested in this build. No separate Accounts page exists.

## Using North

1. Select a destination folder in Overview.
2. Paste a Drive folder link and choose **Analyze folder**.
3. Select **Start**. Let the app continue while you work. Keep the Mac and destination drive available.
4. If transfers fail temporarily or a network connection drops, North automatically retries without losing completed bytes. If the app closes unexpectedly, it restores the session on the next launch. Permanent access errors remain visible under **Issues**.

`.northpart` and `.northpart.json` are temporary resume files, not errors or junk. **Do not manually delete them.** The app removes them when each file completes.

## Build the macOS app

Dependencies: Node.js 22+, stable Rust, Xcode Command Line Tools, macOS.

```sh
npm install
zsh scripts/build-dmg.command
```

The build creates `dist/NorthDriveDownloader-0.2.0-universal-adhoc.dmg`, with both Apple Silicon and Intel targets. GitHub Actions also builds the universal DMG on changes proposed through pull requests.

## Reliability limits

- No software can bypass a Drive owner's permissions, Google's per-file download limits, API quota restrictions, revoked access, or full/disconnected destination disks.
- Temporary failures are retried for an extended period with increasing delays; permanent issues require a user to correct access or credentials and then press Resume.
- Existing completed files with a published MD5 are verified before being skipped; conflicting unverified files are never overwritten silently.
- Google Workspace exports are regenerated rather than range-resumed, because the Drive export API does not support byte ranges.
- The application includes source code and an automated macOS build workflow. **Live end-to-end tests against a 30+ GB folder and a signed/notarized production release have not yet been performed.** Do not treat the PR as a verified production installer until its build and real-world transfer tests pass.

## No bundled credentials

On a clean installation, all Google credential fields start blank. The app stores user-provided API settings locally in the application's configuration, and user-authorized OAuth refresh tokens in macOS Keychain. The session recovery file contains only the folder plan and local destination, not Google API keys or tokens.
