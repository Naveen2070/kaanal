// shadcn/ui's Slider over Radix (spec 7), restyled with our tokens: one thumb, a 24px hit area
// (WCAG 2.5.8), the range in accent on a well track. Radix gives the keys (arrows, PageUp/PageDown,
// Home, End) and role="slider"; the label and the value in words go on the thumb, which is what
// takes focus.

import * as SliderPrimitive from "@radix-ui/react-slider";
import type { ComponentProps } from "react";

export function Slider({
  className = "",
  "aria-label": label,
  "aria-valuetext": valueText,
  ...props
}: ComponentProps<typeof SliderPrimitive.Root> & { "aria-valuetext"?: string }) {
  return (
    <SliderPrimitive.Root
      className={`relative flex h-6 w-full touch-none items-center select-none data-disabled:opacity-60 ${className}`}
      {...props}
    >
      <SliderPrimitive.Track className="relative h-1.5 grow overflow-hidden rounded-full bg-well">
        <SliderPrimitive.Range className="absolute h-full bg-accent" />
      </SliderPrimitive.Track>
      <SliderPrimitive.Thumb
        aria-label={label}
        aria-valuetext={valueText}
        className="block size-6 rounded-full border-2 border-accent bg-sheet focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
      />
    </SliderPrimitive.Root>
  );
}
