import { cn } from '../lib/utils.ts';

/**
 * The product mark: two arrows passing each other — traffic entering and
 * leaving through one point. Rendered on a violet tile to match the accent.
 */
export function BrandMark({ size = 15, className }: { size?: number; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn('inline-flex size-6 items-center justify-center rounded-md bg-primary text-primary-foreground shadow-sm', className)}
    >
      <svg
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M5 9h13" />
        <path d="M15 6l3 3-3 3" />
        <path d="M19 15H6" />
        <path d="M9 12l-3 3 3 3" />
      </svg>
    </span>
  );
}
