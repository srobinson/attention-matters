"use client";

/**
 * Parses text containing <salient>...</salient> tags and renders
 * the tagged portions with gold highlighting.
 */
export function SalientText({ text }: { text: string }) {
  const parts = parseSalientTags(text);

  return (
    <>
      {parts.map((part, i) =>
        part.salient ? (
          <span
            key={i}
            className="font-medium"
            style={{ color: "var(--color-salient)" }}
          >
            {part.text}
          </span>
        ) : (
          <span key={i}>{part.text}</span>
        )
      )}
    </>
  );
}

interface TextPart {
  text: string;
  salient: boolean;
}

function parseSalientTags(input: string): TextPart[] {
  const parts: TextPart[] = [];
  let remaining = input;

  while (remaining.length > 0) {
    const openIdx = remaining.indexOf("<salient>");
    if (openIdx === -1) {
      parts.push({ text: remaining, salient: false });
      break;
    }

    // Text before the tag
    if (openIdx > 0) {
      parts.push({ text: remaining.slice(0, openIdx), salient: false });
    }

    const afterOpen = remaining.slice(openIdx + 9);
    const closeIdx = afterOpen.indexOf("</salient>");

    if (closeIdx === -1) {
      // Unclosed tag: treat as salient (still streaming)
      parts.push({ text: afterOpen, salient: true });
      break;
    }

    parts.push({ text: afterOpen.slice(0, closeIdx), salient: true });
    remaining = afterOpen.slice(closeIdx + 10);
  }

  return parts;
}

/**
 * Strip salient tags from text for clean display.
 */
export function stripSalientTags(text: string): string {
  return text.replace(/<\/?salient>/g, "");
}
