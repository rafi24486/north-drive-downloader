# North Drive Downloader 1.0.0 — Public-Only Edition

A native macOS Google Drive folder downloader for **publicly shared folders**, using a dark/pink Tauri UI. No Google account sign-in, OAuth client ID, account selector, or login screen.

## How to use

1. Enable the **Google Drive API** in a Google Cloud project and create a Google API key.
2. In the app, open **Settings**, enter the API key, and save. This is **not a Google account login**. Google's official `files.list` API requires an API key to enumerate publicly shared folders.
3. In Google Drive, set the folder access to **Anyone with the link** and ensure downloads are allowed.
4. Paste the folder URL, choose a Mac destination folder, and select **Analyze folder → Start**.
5. If a file fails, view its error under **Downloads → Issues**. Choose **Retry failed** to continue a resumable file rather than downloading the same bytes again. Use the same destination after restarting the app.

**Private or organization-restricted folders are intentionally not supported.** The app does not provide a workaround for owners' download restrictions or API quota limits.

## Reliability features

- Streaming downloads with checked HTTP content lengths, byte-range validation, partial-file resume and safe restarts if a server ignores Range.
- Network backoff, retry for transient Google Drive rate limits/server errors and one clean retry on MD5 mismatch.
- Preservation of unfinished `.northpart` files and metadata across restarts. Completed Google Workspace exports have SHA-256 receipts in the hidden `.north-drive-state` folder; incomplete exports must restart.
- Transfer cancellation and pauses, failed-file retry, real error detail in Issues.
- Default **2 parallel transfers** to reduce throttling on unstable connections. Adjustable from 1 to 8 in Settings.
- Handles resource-key links, nested folders, duplicate filenames, and files with download restrictions (the latter are skipped with a warning).

## macOS DMG

Go to **Actions → Build macOS Universal DMG** in this repository, open a **successful run for the desired commit**, and download the **NorthDriveDownloader-macOS-Universal-DMG** artifact. Unzip it and open the DMG to install the app.

To build on macOS (Node.js 22+, stable Rust, Xcode Command Line Tools):

```sh
zsh scripts/build-dmg.command
```

The DMG contains `North Drive Downloader.app` and an Applications shortcut. It is **ad-hoc signed, not Apple-notarized**. Use the standard macOS Open Anyway option only if you trust the source and build; do not disable Gatekeeper globally.

## QA and limitations

GitHub Actions runs `node --check ui/app.js`, Rust regression tests, a native universal macOS build, and a DMG-content check. These checks cannot prove that every Google Drive folder or connection will work: **real-world downloads with the affected public folder are still needed**. Do not claim zero bugs or 100% download success until that testing is done.

If an older version stored OAuth data in macOS Keychain, this version does not read or use it. Previously saved API keys remain in the local app preferences so upgrading doesn't require re-entering the key.

The Google Drive API key should be restricted to the Google Drive API in Google Cloud and kept private; never commit it to the repository.
