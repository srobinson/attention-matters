"use client";

import { Upload } from "lucide-react";

interface UploadButtonProps {
  onClick: () => void;
}

/**
 * Upload trigger button for the settings bar header.
 * Opens the upload modal when clicked.
 */
export function UploadButton({ onClick }: UploadButtonProps) {
  return (
    <button
      onClick={onClick}
      className="flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:opacity-80"
      style={{ color: "var(--color-text-secondary)" }}
      aria-label="Upload document"
      title="Upload document"
    >
      <Upload className="h-4 w-4" />
    </button>
  );
}
