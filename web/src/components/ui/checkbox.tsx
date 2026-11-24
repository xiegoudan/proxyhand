import * as React from 'react'
import { cn } from '../../lib/utils'

export const Checkbox = React.forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => (
    <input
      ref={ref}
      type="checkbox"
      className={cn(
        'size-3.5 shrink-0 cursor-pointer appearance-none rounded-[4px] border border-input bg-card transition-colors',
        'checked:border-primary checked:bg-primary checked:after:block checked:after:text-[9px] checked:after:leading-[13px] checked:after:text-primary-foreground checked:after:content-[\"✓\"]',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40 disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    />
  ),
)
Checkbox.displayName = 'Checkbox'
