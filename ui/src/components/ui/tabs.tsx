// shadcn/ui's Tabs over Radix (spec 6, 7): roving tabindex, arrow keys, aria-controls and
// aria-selected come from Radix. One selected tab gets the accent underline.

import * as TabsPrimitive from "@radix-ui/react-tabs";
import type { ComponentProps } from "react";

export const Tabs = TabsPrimitive.Root;

export function TabsList({ className = "", ...props }: ComponentProps<typeof TabsPrimitive.List>) {
  return (
    <TabsPrimitive.List
      className={`flex shrink-0 border-b border-border ${className}`}
      {...props}
    />
  );
}

export function TabsTrigger({
  className = "",
  ...props
}: ComponentProps<typeof TabsPrimitive.Trigger>) {
  return (
    <TabsPrimitive.Trigger
      className={`flex-1 border-b-2 border-transparent px-2 py-2 text-sm text-muted focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent data-[state=active]:border-accent data-[state=active]:font-medium data-[state=active]:text-text ${className}`}
      {...props}
    />
  );
}

export function TabsContent({
  className = "",
  ...props
}: ComponentProps<typeof TabsPrimitive.Content>) {
  return (
    <TabsPrimitive.Content className={`min-h-0 flex-1 overflow-y-auto ${className}`} {...props} />
  );
}
