import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SettingsView } from "./SettingsView";

describe("Settings interoperability controls", () => {
  it("keeps exports explicit and exposes local resilience actions", () => {
    const exportSidecars = vi.fn();
    const rescan = vi.fn();
    render(<SettingsView
      status={{ configured: true, libraryRoot: "D:\\Photo Library", counts: { total: 1, keep: 0, undecided: 1, discard: 0 } }}
      health={{ localAiAvailable: false, serviceReachable: false, localAiBusy: false, localAiDetail: "Offline", localAiState: "service_not_running", localAiUrl: "http://127.0.0.1:7868", analysisModelInstalled: false, analysisAvailable: false, analysisDetail: "Unavailable" }}
      busy={false} selectedAssetName="photo.jpg" onIntegrity={vi.fn()} onBackup={vi.fn()} onRestore={vi.fn()} onRebuild={vi.fn()} onDiagnostics={vi.fn()} onRefreshAi={vi.fn().mockResolvedValue(undefined)} onConfigureAi={vi.fn().mockResolvedValue(undefined)}
      onExportSidecars={exportSidecars} onImportSidecar={vi.fn()} onExportPortableCatalogue={vi.fn()} onRescanLibrary={rescan} onRelinkSelected={vi.fn()} onAddFolderWatch={vi.fn()} onDisableFolderWatches={vi.fn()} onShowFolderWatchEvents={vi.fn()}
    />);
    fireEvent.click(screen.getByRole("button", { name: "Export selected XMP" }));
    expect(exportSidecars).toHaveBeenCalledWith("selected", false);
    fireEvent.click(screen.getByRole("button", { name: "Scan library" }));
    expect(rescan).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Watch a folder…" })).toBeVisible();
    expect(screen.getByText(/nothing is imported, removed or relinked automatically/i)).toBeVisible();
  });
});
