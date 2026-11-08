# Privacy Policy v2

**Last Updated: November 8, 2026**

When using **Aurelia**, your privacy and creative ownership are absolute priorities. This Privacy Policy explains our data practices, architecture, and commitments.

---

## 1. Zero Telemetry & Local-First Philosophy

- **No Data Collection**: Aurelia does not collect, record, track, or transmit any personal information, telemetry, usage statistics, diagnostic reports, or crash logs.
- **100% Offline**: Aurelia does not directly initiate outbound network requests or communicate with remote servers. Optional integrations, such as Discord Rich Presence, communicate with locally installed applications that may independently access their own online services.
- **Your Artwork Stays Yours**: All artwork, project files (`.aur`), image exports, palettes, and configurations remain strictly on your local device.

---

## 2. Optional Discord Rich Presence

Discord Rich Presence is **off by default**. Nothing is sent to Discord unless you explicitly enable it from **Window → Enable Discord Rich Presence**.

- **Local IPC only**: Aurelia talks to the Discord desktop client through a local named pipe. It uses no Discord API token, bot, webhook, or outbound network connection.
- **Generic by default**: the activity contains only "Painting"/"Idle", "In Aurelia", and the Aurelia logo.
- **No project data by default**: project names, file paths, layer names, text content, and artwork are never included in the default payload.
- **Project name is a separate opt-in**: enabling "Show project name" sends only the project's display name (the file stem, e.g. `client-work`), never its path.
- **Artwork previews are not implemented** in this version.
- **Clearing**: disabling the feature or closing Aurelia clears the activity immediately.
- **Enforced by test**: a unit test asserts that the default payload contains no document name or path.

Discord must be running for the activity to appear. If it is not, Aurelia silently does nothing.

---

## 3. File System Access

Aurelia interacts with your local file system solely to fulfill user-initiated actions:

- **Saving & Loading Projects**: When you explicitly choose to Save or Open a `.aur` project file.
- **Exporting Artwork**: When you export raster image files (e.g., PNG).
- **Importing Reference Images**: When you load a reference image into the dock.

The application never scans, indexes, or accesses files outside the specific directories you select in the system file dialog.

---

## 4. Third-Party Services & Dependencies

- **Third-Party SDKs**: Aurelia does not integrate third-party analytics, advertising, tracking, or cloud-storage SDKs.
- **Pure Native Build**: The codebase is compiled directly to native binary code with zero external telemetry dependencies.

---

## 5. Updates & Security

Because Aurelia does not communicate with external servers, it does not perform automated background updates or background checks. Updates are distributed via official repository releases on GitHub.

---

## 6. Privacy Architecture

Aurelia is designed as a local-first desktop application. Its image-processing engine, document system, rendering pipeline, and editing operations execute locally on your device.

Project contents are not uploaded to external services, processed by remote servers, or synchronized to cloud infrastructure.

Optional integrations are isolated from the core editing workflow and require explicit user activation.

Aurelia does not require an online account, subscription authentication, or remote service to open, edit, save, or export artwork.

---

## 7. Contact & Inquiries

If you have any questions regarding this Privacy Policy or Aurelia's privacy architecture, please contact:

- **Maintainer**: `trollologistog@gmail.com`
- **GitHub**: [https://github.com/LeTrollologist/Aurelia](https://github.com/LeTrollologist/Aurelia)
