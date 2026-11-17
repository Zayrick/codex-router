import { useState } from "react";

import type { ModelStats, UsageAnalytics } from "@/admin-api";
import { Meter, RangeBar } from "@/components/app/bars";
import { Tag } from "@/components/app/badges";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { VIZ } from "@/components/app/viz";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatCost, formatCount, formatPercent, formatSeconds, formatTokens } from "@/lib/format";
import { cn } from "@/lib/utils";

import { NoData, SignedDelta } from "./common";
import { cacheRate, modelColors } from "./metrics";

type Sort = "tokens" | "cost" | "requests";

function failureRate(model: ModelStats): number | null {
	return model.requests > 0 ? (model.failed + model.interrupted) / model.requests : null;
}

/** Figma "card · 模型效率": per-model usage, cost and speed. */
export function ModelEfficiencyCard({ analytics, title = "模型效率", className = "" }: { analytics: UsageAnalytics; title?: string; className?: string }) {
	const [sort, setSort] = useState<Sort>("tokens");
	const colors = modelColors(analytics);
	const totalTokens = analytics.totals.totalTokens;
	const models = [...analytics.models].sort((left, right) =>
		sort === "cost" ? right.costUsd - left.costUsd : sort === "requests" ? right.requests - left.requests : right.totalTokens - left.totalTokens,
	);
	const maxSpeed = Math.max(1, ...models.flatMap((model) => [model.outputSpeedFirstHalf ?? 0, model.outputSpeedSecondHalf ?? 0, model.outputSpeed ?? 0]));
	const unpriced = analytics.unpricedModels;
	return (
		<SectionCard
			actions={
				<Segmented
					label="排序"
					onChange={setSort}
					options={[
						{ value: "tokens", label: "按 Token" },
						{ value: "cost", label: "按成本" },
						{ value: "requests", label: "按请求" },
					]}
					value={sort}
				/>
			}
			className={className}
			contentClassName="gap-2"
			description="输出速度 = 输出 Token ÷ 总延迟"
			title={title}
		>
			{models.length === 0 ? (
				<NoData />
			) : (
				<div className="overflow-x-auto">
					<Table className="min-w-[960px] text-[13px]">
						<TableHeader>
							<TableRow className="hover:bg-transparent [&>th]:h-8 [&>th]:text-[11.5px] [&>th]:font-medium [&>th]:text-faint">
								<TableHead className="w-56">模型</TableHead>
								<TableHead className="text-right">请求</TableHead>
								<TableHead className="w-44">Token · 份额</TableHead>
								<TableHead className="text-right">缓存命中</TableHead>
								<TableHead className="text-right">每请求输出</TableHead>
								<TableHead className="text-right">每请求成本</TableHead>
								<TableHead className="text-right">成本</TableHead>
								<TableHead className="w-32">输出速度 t/s</TableHead>
								<TableHead className="text-right">P95 首字</TableHead>
								<TableHead className="text-right">失败率</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{models.map((model) => {
								const color = colors.get(model.model) ?? VIZ.gray;
								const share = totalTokens > 0 ? model.totalTokens / totalTokens : 0;
								const failure = failureRate(model);
								const redirect = model.redirects[0];
								return (
									<TableRow className="hover:bg-transparent [&>td]:py-3" key={model.model}>
										<TableCell>
											<div className="flex flex-col items-start gap-1">
												<span className="flex items-center gap-2 font-mono text-[13px] font-bold">
													<span aria-hidden="true" className="size-2 rounded-[2px]" style={{ background: color }} />
													{model.model}
												</span>
												{redirect ? (
													<Tag className="rounded-md bg-primary-soft font-normal">
														{formatCount(model.redirects.reduce((sum, entry) => sum + entry.requests, 0))} 次由 {redirect.name} 重定向
													</Tag>
												) : null}
											</div>
										</TableCell>
										<TableCell className="text-right font-medium tabular-nums">{formatCount(model.requests)}</TableCell>
										<TableCell>
											<div className="flex flex-col gap-1">
												<span className="flex items-baseline gap-2">
													<span className="font-semibold tabular-nums">{formatTokens(model.totalTokens)}</span>
													<span className="text-[11.5px] text-faint tabular-nums">{formatPercent(share)}</span>
												</span>
												<Meter color={color} height={4} ratio={share} />
											</div>
										</TableCell>
										<TableCell className="text-right tabular-nums">{formatPercent(cacheRate(model))}</TableCell>
										<TableCell className="text-right tabular-nums">{formatCount(model.requests ? model.outputTokens / model.requests : null)}</TableCell>
										<TableCell className={cn("text-right tabular-nums", !model.priced && "text-faint")}>
											{model.priced && model.requests ? formatCost(model.costUsd / model.requests) : "—"}
										</TableCell>
										<TableCell className="text-right font-semibold tabular-nums">
											{model.priced ? formatCost(model.costUsd) : <Tag tone="warning">未定价</Tag>}
										</TableCell>
										<TableCell>
											<div className="flex flex-col gap-1">
												<RangeBar
													color={color}
													from={Math.min(model.outputSpeedFirstHalf ?? 0, model.outputSpeedSecondHalf ?? 0) || null}
													marker={model.outputSpeedSecondHalf}
													max={maxSpeed}
													to={Math.max(model.outputSpeedFirstHalf ?? 0, model.outputSpeedSecondHalf ?? 0) || null}
												/>
												<span className="flex items-baseline gap-1">
													<span className="text-[12.5px] font-semibold tabular-nums">{model.outputSpeed?.toFixed(1) ?? "—"}</span>
													<SignedDelta
														value={
															model.outputSpeedFirstHalf !== null && model.outputSpeedSecondHalf !== null
																? model.outputSpeedSecondHalf - model.outputSpeedFirstHalf
																: null
														}
													/>
												</span>
											</div>
										</TableCell>
										<TableCell className="text-right tabular-nums">{formatSeconds(model.ttft.p95, 1)}</TableCell>
										<TableCell className={cn("text-right tabular-nums", (failure ?? 0) > 0.03 && "font-semibold text-danger")}>{formatPercent(failure)}</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				</div>
			)}
			<div className="flex flex-wrap items-center gap-x-4 gap-y-1 pt-2 text-[11.5px] text-muted-foreground">
				<span>速度条：前半段 → 后半段（竖线）</span>
				{unpriced.length ? (
					<span className="text-warning">
						{unpriced.map((model) => model.model).join("、")} 未定价，不计入成本（{formatTokens(unpriced.reduce((sum, model) => sum + model.totalTokens, 0))} Token）
					</span>
				) : null}
			</div>
		</SectionCard>
	);
}
