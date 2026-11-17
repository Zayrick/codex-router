import type { ReactNode } from "react";

import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { cn } from "@/lib/utils";

/**
 * Figma "card · …" frames: 14px radius, 24px padding, header with title,
 * description and right-aligned actions.
 */
export function SectionCard({
	title,
	description,
	actions,
	children,
	className,
	contentClassName,
	id,
}: {
	title: ReactNode;
	description?: ReactNode;
	actions?: ReactNode;
	children?: ReactNode;
	className?: string;
	contentClassName?: string;
	id?: string;
}) {
	return (
		<Card className={cn("gap-4 py-6 [--card-spacing:--spacing(6)]", className)} id={id}>
			<CardHeader className="gap-1">
				<CardTitle className="text-[15px] leading-5 font-semibold tracking-[-0.01em]">{title}</CardTitle>
				{description ? <CardDescription className="text-[12.5px] leading-5">{description}</CardDescription> : null}
				{actions ? <CardAction className="flex items-center gap-2">{actions}</CardAction> : null}
			</CardHeader>
			{children !== undefined ? <CardContent className={cn("flex flex-col gap-4", contentClassName)}>{children}</CardContent> : null}
		</Card>
	);
}

/** Uppercase-free small heading used inside cards ("按上游账户", "错误码"). */
export function SubHeading({ children, aside, className }: { children: ReactNode; aside?: ReactNode; className?: string }) {
	return (
		<div className={cn("flex items-center justify-between gap-3", className)}>
			<h3 className="text-[12.5px] leading-5 font-semibold">{children}</h3>
			{aside ? <span className="text-xs text-faint">{aside}</span> : null}
		</div>
	);
}
