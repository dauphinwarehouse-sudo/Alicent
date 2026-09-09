export function makeLargeMarkdown(wordCount = 200_000): string {
  const wordsPerParagraph = 100;
  const words = [
    "Алиса",
    "видит",
    "далёкий",
    "город",
    "и",
    "пишет",
    "новую",
    "главу",
  ];
  const paragraphs: string[] = ["# Большой документ"];
  for (let offset = 0; offset < wordCount; offset += wordsPerParagraph) {
    const length = Math.min(wordsPerParagraph, wordCount - offset);
    const paragraph = Array.from(
      { length },
      (_, index) => words[(offset + index) % words.length],
    ).join(" ");
    paragraphs.push(paragraph);
  }
  return `${paragraphs.join("\n\n")}\n`;
}
