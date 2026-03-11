/**
 * Shared utilities for memory components.
 * Category colors map AM recall categories to design tokens.
 */

export function getCategoryColor(category: string): string {
  switch (category) {
    case "Conscious":
      return "var(--color-conscious)";
    case "Subconscious":
      return "var(--color-subconscious)";
    case "Novel":
      return "var(--color-novel)";
    default:
      return "var(--color-text-secondary)";
  }
}
