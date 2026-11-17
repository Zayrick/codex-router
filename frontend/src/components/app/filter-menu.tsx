import { ChevronDownIcon } from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuCheckboxItem,
	DropdownMenuContent,
	DropdownMenuLabel,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export interface FilterOption {
	value: string;
	label: ReactNode;
	/** Plain-text label for the trigger summary. */
	text?: string;
	hint?: ReactNode;
}

/**
 * Figma Log/Filter: "字段 值 ⌄" trigger, active (soft fill) when a condition is
 * set, opening a DropdownMenu with checkbox items (multi) or radio items.
 */
export function FilterMenu({
	label,
	options,
	selected,
	onChange,
	multiple = true,
	allLabel = "全部",
	className,
	align = "start",
}: {
	label: string;
	options: FilterOption[];
	selected: string[];
	onChange: (values: string[]) => void;
	multiple?: boolean;
	allLabel?: string;
	className?: string;
	align?: "start" | "end";
}) {
	const active = selected.length > 0;
	const summary = !active
		? allLabel
		: selected.length === 1
			? (options.find((option) => option.value === selected[0])?.text ?? selected[0])
			: `${selected.length} 项`;
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					className={cn(
						"h-[30px] gap-1 rounded-lg px-2.5 text-[12.5px] font-normal text-muted-foreground",
						active && "border-primary-border bg-primary-soft hover:bg-primary-soft",
						className,
					)}
					size="sm"
					type="button"
					variant="outline"
				>
					{label}
					<span className={cn("max-w-40 truncate text-foreground", active ? "font-bold" : "font-medium")}>{summary}</span>
					<ChevronDownIcon className="size-3.5 text-muted-foreground" data-icon="inline-end" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align={align} className="max-h-80 min-w-48">
				<DropdownMenuLabel className="text-xs text-muted-foreground">{label}</DropdownMenuLabel>
				{multiple ? (
					<>
						<DropdownMenuCheckboxItem checked={!active} onCheckedChange={() => onChange([])} onSelect={(event) => event.preventDefault()}>
							{allLabel}
						</DropdownMenuCheckboxItem>
						<DropdownMenuSeparator />
						{options.map((option) => (
							<DropdownMenuCheckboxItem
								checked={selected.includes(option.value)}
								key={option.value}
								onCheckedChange={(checked) =>
									onChange(checked ? [...selected, option.value] : selected.filter((value) => value !== option.value))
								}
								onSelect={(event) => event.preventDefault()}
							>
								<span className="flex min-w-0 flex-1 items-center justify-between gap-3">
									<span className="truncate">{option.label}</span>
									{option.hint ? <span className="text-xs text-faint">{option.hint}</span> : null}
								</span>
							</DropdownMenuCheckboxItem>
						))}
					</>
				) : (
					<DropdownMenuRadioGroup onValueChange={(value) => onChange(value ? [value] : [])} value={selected[0] ?? ""}>
						<DropdownMenuRadioItem value="">{allLabel}</DropdownMenuRadioItem>
						<DropdownMenuSeparator />
						{options.map((option) => (
							<DropdownMenuRadioItem key={option.value} value={option.value}>
								<span className="flex min-w-0 flex-1 items-center justify-between gap-3">
									<span className="truncate">{option.label}</span>
									{option.hint ? <span className="text-xs text-faint">{option.hint}</span> : null}
								</span>
							</DropdownMenuRadioItem>
						))}
					</DropdownMenuRadioGroup>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
