export const formatDate = (iso: string, options: Intl.DateTimeFormatOptions = { day: "numeric", month: "short", year: "numeric" }) =>
  new Intl.DateTimeFormat("en-GB", options).format(new Date(iso));

export const displayIntent = (value: string) => value.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());

export const decisionLabel = { keep: "Keep", undecided: "Undecided", discard: "Discard" } as const;
