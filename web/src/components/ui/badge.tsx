import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '../../lib/utils'

const badgeVariants = cva(
  'inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium leading-none',
  {
    variants: {
      variant: {
        default: 'bg-primary/15 text-primary',
        secondary: 'bg-secondary text-secondary-foreground',
        outline: 'border border-border text-muted-foreground',
        ok: 'bg-[hsl(152_55%_46%_/_0.16)] text-[hsl(152_55%_56%)]',
        redirect: 'bg-[hsl(38_92%_55%_/_0.16)] text-[hsl(38_92%_62%)]',
        client: 'bg-[hsl(28_90%_58%_/_0.16)] text-[hsl(28_90%_64%)]',
        server: 'bg-[hsl(0_72%_58%_/_0.16)] text-[hsl(0_72%_66%)]',
        pending: 'bg-muted text-muted-foreground',
      },
    },
    defaultVariants: { variant: 'secondary' },
  },
)

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement>, VariantProps<typeof badgeVariants> {}

export function Badge({ className, variant, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ variant }), className)} {...props} />
}
