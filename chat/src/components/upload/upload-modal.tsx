"use client";

import { useState, useRef, useCallback, useEffect } from "react";
import { X, FileText, CheckCircle2, AlertCircle, Loader2 } from "lucide-react";
import { amIngest, amImport } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";

type UploadState =
  | { status: "idle" }
  | { status: "reading"; fileName: string }
  | { status: "uploading"; fileName: string }
  | { status: "success"; fileName: string; message: string }
  | { status: "error"; fileName: string; message: string };

const ACCEPTED_EXTENSIONS = [".txt", ".md", ".json"];
const ACCEPT_STRING = ".txt,.md,.json";
const MAX_FILE_SIZE_BYTES = 5 * 1024 * 1024; // 5MB
const MAX_FILE_SIZE_LABEL = "5 MB";

/**
 * Detects whether a JSON object is an AM v0.7.2 export format.
 * The export format contains a top-level "version" field set to "0.7.2".
 */
function isAMExportFormat(parsed: unknown): boolean {
  if (typeof parsed !== "object" || parsed === null) return false;
  const obj = parsed as Record<string, unknown>;
  return obj.version === "0.7.2";
}

interface UploadModalProps {
  open: boolean;
  onClose: () => void;
  onIngestComplete?: () => void;
}

/**
 * Modal dialog for uploading documents to AM.
 * Supports .txt, .md (ingested as text), and .json (auto-detects AM export format).
 */
export function UploadModal({ open, onClose, onIngestComplete }: UploadModalProps) {
  const [state, setState] = useState<UploadState>({ status: "idle" });
  const [dragOver, setDragOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const modalRef = useRef<HTMLDivElement>(null);

  // Reset state when modal opens
  useEffect(() => {
    if (open) {
      setState({ status: "idle" });
      setDragOver(false);
    }
  }, [open]);

  // Focus trap and escape key
  useEffect(() => {
    if (!open) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    modalRef.current?.focus();

    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, onClose]);

  const processFile = useCallback(async (file: File) => {
    // Size limit check (client-side, before any upload)
    if (file.size > MAX_FILE_SIZE_BYTES) {
      const sizeMB = (file.size / (1024 * 1024)).toFixed(1);
      setState({
        status: "error",
        fileName: file.name,
        message: `File too large (${sizeMB} MB). Maximum size is ${MAX_FILE_SIZE_LABEL}.`,
      });
      return;
    }

    const ext = file.name.substring(file.name.lastIndexOf(".")).toLowerCase();
    if (!ACCEPTED_EXTENSIONS.includes(ext)) {
      setState({
        status: "error",
        fileName: file.name,
        message: `Unsupported file type: ${ext}. Accepted: ${ACCEPTED_EXTENSIONS.join(", ")}`,
      });
      return;
    }

    setState({ status: "reading", fileName: file.name });

    try {
      const text = await file.text();
      const apiKey = loadSettings().apiKey || undefined;

      setState({ status: "uploading", fileName: file.name });

      if (ext === ".json") {
        let parsed: unknown;
        try {
          parsed = JSON.parse(text);
        } catch {
          setState({
            status: "error",
            fileName: file.name,
            message: "Invalid JSON file. Could not parse content.",
          });
          return;
        }

        if (isAMExportFormat(parsed)) {
          // AM export format: use /api/am/import
          const result = await amImport({ state: parsed }, apiKey);
          setState({
            status: "success",
            fileName: file.name,
            message: `Imported ${result.episodes} episodes, ${result.neighborhoods} topic clusters, ${result.occurrences} occurrences`,
          });
          onIngestComplete?.();
          return;
        }
      }

      // Default path: ingest as text
      const result = await amIngest({ text, name: file.name }, apiKey);
      setState({
        status: "success",
        fileName: file.name,
        message: `Ingested "${result.episode_name}" with ${result.neighborhoods} topic clusters`,
      });
      onIngestComplete?.();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Upload failed";
      setState({
        status: "error",
        fileName: file.name,
        message,
      });
    }
  }, [onIngestComplete]);

  const handleFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (file) processFile(file);
      // Reset input so re-selecting the same file triggers onChange
      e.target.value = "";
    },
    [processFile]
  );

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const file = e.dataTransfer.files[0];
      if (file) processFile(file);
    },
    [processFile]
  );

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
  }, []);

  if (!open) return null;

  const isProcessing = state.status === "reading" || state.status === "uploading";

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-50"
        style={{ background: "rgba(0, 0, 0, 0.6)" }}
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Modal */}
      <div
        ref={modalRef}
        role="dialog"
        aria-modal="true"
        aria-label="Upload document"
        tabIndex={-1}
        className="fixed left-1/2 top-1/2 z-50 w-[90vw] max-w-md -translate-x-1/2 -translate-y-1/2 rounded-lg border p-6 outline-none"
        style={{
          borderColor: "var(--color-border)",
          background: "var(--color-surface)",
        }}
      >
        {/* Header */}
        <div className="mb-4 flex items-center justify-between">
          <h2
            className="text-sm font-medium"
            style={{ color: "var(--color-text-primary)" }}
          >
            Upload Document
          </h2>
          <button
            onClick={onClose}
            className="flex h-6 w-6 items-center justify-center rounded transition-colors hover:opacity-80"
            style={{ color: "var(--color-text-secondary)" }}
            aria-label="Close upload dialog"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Drop zone */}
        <div
          className="flex cursor-pointer flex-col items-center justify-center rounded-lg border-2 border-dashed px-4 py-8 transition-colors"
          style={{
            borderColor: dragOver
              ? "var(--color-salient)"
              : "var(--color-border)",
            background: dragOver
              ? "var(--color-surface-raised)"
              : "transparent",
          }}
          onClick={() => !isProcessing && fileInputRef.current?.click()}
          onDrop={handleDrop}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
          role="button"
          tabIndex={0}
          aria-label="Drop a file here or click to browse"
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              fileInputRef.current?.click();
            }
          }}
        >
          <FileText
            className="mb-2 h-8 w-8"
            style={{ color: "var(--color-text-secondary)" }}
          />
          <p
            className="text-sm"
            style={{ color: "var(--color-text-primary)" }}
          >
            Drop a file here or click to browse
          </p>
          <p
            className="mt-1 text-xs"
            style={{ color: "var(--color-text-secondary)" }}
          >
            .txt, .md, .json (max {MAX_FILE_SIZE_LABEL})
          </p>
        </div>

        <input
          ref={fileInputRef}
          type="file"
          accept={ACCEPT_STRING}
          onChange={handleFileSelect}
          className="hidden"
          aria-hidden="true"
        />

        {/* Status display */}
        {state.status !== "idle" && (
          <div
            className="mt-4 flex items-start gap-2 rounded-md border px-3 py-2"
            style={{
              borderColor:
                state.status === "success"
                  ? "var(--color-novel)"
                  : state.status === "error"
                    ? "#ef4444"
                    : "var(--color-border)",
              background: "var(--color-surface-raised)",
            }}
            role={state.status === "error" ? "alert" : "status"}
          >
            {isProcessing && (
              <Loader2
                className="mt-0.5 h-4 w-4 flex-shrink-0 animate-spin"
                style={{ color: "var(--color-salient)" }}
              />
            )}
            {state.status === "success" && (
              <CheckCircle2
                className="mt-0.5 h-4 w-4 flex-shrink-0"
                style={{ color: "var(--color-novel)" }}
              />
            )}
            {state.status === "error" && (
              <AlertCircle
                className="mt-0.5 h-4 w-4 flex-shrink-0"
                style={{ color: "#ef4444" }}
              />
            )}
            <div className="min-w-0 flex-1">
              <p
                className="truncate text-xs font-medium"
                style={{ color: "var(--color-text-primary)" }}
              >
                {state.fileName}
              </p>
              {state.status === "reading" && (
                <p
                  className="text-[11px]"
                  style={{ color: "var(--color-text-secondary)" }}
                >
                  Reading file...
                </p>
              )}
              {state.status === "uploading" && (
                <p
                  className="text-[11px]"
                  style={{ color: "var(--color-text-secondary)" }}
                >
                  Ingesting into memory...
                </p>
              )}
              {(state.status === "success" || state.status === "error") && (
                <p
                  className="text-[11px]"
                  style={{
                    color:
                      state.status === "success"
                        ? "var(--color-novel)"
                        : "#ef4444",
                  }}
                >
                  {state.message}
                </p>
              )}
            </div>
          </div>
        )}

        {/* JSON hint */}
        <p
          className="mt-3 text-[11px]"
          style={{ color: "var(--color-text-secondary)" }}
        >
          JSON files with AM export format (v0.7.2) are automatically detected and imported.
          Other files are ingested as new episodes.
        </p>
      </div>
    </>
  );
}
