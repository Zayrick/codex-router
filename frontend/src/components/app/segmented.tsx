import type { ReactNode } from "react";

import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { cn } from "@/lib/utils";

export interface SegmentedOption<T extends string> {
	value: T;
	label: ReactNode;
	/** Count shown after the label ("失败 9"). */
	count?: ReactNode;
	disabled?: boolean;
}

/**
 * Figma "segmented" / Toggle Item: ToggleGroupItem variant=outline joined with
 * spacing 0. `sm` (28px) sits in card headers, `default` (32px) in toolbars.
 */
export function Segmented<T extends string>({
	value,
	onChange,
	options,
	size = "sm",
	className,
	label,
}: {
	value: T;
	onChange: (value: T) => void;
	options: SegmentedOption<T>[];
	size?: "sm" | "default";
	className?: string;
	label?: string;
}) {
	return (
		<ToggleGroup
			aria-label={label}
			className={cn("bg-card", className)}
			onValueChange={(next) => {
				if (next) onChange(next as T);
			}}
			size={size}
			spacing={0}
			type="single"
			value={value}
			variant="outline"
		>
			{options.map((option) => (
				<ToggleGroupItem
					className={cn(
						"font-medium text-muted-foreground data-[state=on]:bg-surface-subtle data-[state=on]:text-foreground",
						size === "sm" ? "h-7 px-2.5 text-[12.5px]" : "h-8 px-3 text-[13px]",
					)}
					disabled={option.disabled}
					key={option.value}
					value={option.value}
				>
					{option.label}
					{option.count !== undefined ? <span className="ml-1 text-xs text-faint tabular-nums">{option.count}</span> : null}
				</ToggleGroupItem>
			))}
		</ToggleGroup>
	);
}
