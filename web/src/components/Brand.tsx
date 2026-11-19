/**
 * The product mark: a teal tile with two arrows passing each other — traffic entering and
 * leaving through one point, which is all this tool does.
 */
export function BrandMark() {
  return (
    <span className="brand-mark" aria-hidden="true">
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
        <path d="M5 9h13" />
        <path d="M15 6l3 3-3 3" />
        <path d="M19 15H6" />
        <path d="M9 12l-3 3 3 3" />
      </svg>
    </span>
  );
}
