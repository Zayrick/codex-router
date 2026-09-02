import { useMemo, useState } from "react";
import {
	CircleDollarSignIcon,
	ExternalLinkIcon,
	PlusIcon,
	RefreshCwIcon,
} from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Empty,
	EmptyDescription,
	EmptyHeader,
	EmptyMedia,
	EmptyTitle,
} from "@/components/ui/empty";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import type { ModelPrice } from "./admin-api";

interface ModelPricingCardProps {
	error: string | null;
	loading: boolean;
	prices: ModelPrice[];
	saving: boolean;
	syncing: boolean;
	usedModels: string[];
	onSave: (prices: ModelPrice[]) => void;
	onSync: () => void;
}

export default function ModelPricingCard({
	error,
	loading,
	prices,
	saving,
	syncing,
	usedModels,
	onSave,
	onSync,
}: ModelPricingCardProps) {
	const [drafts, setDrafts] = useState<ModelPrice[]>(prices);
	const [sourcePrices, setSourcePrices] = useState(prices);
	const [newModel, setNewModel] = useState("");

	if (prices !== sourcePrices) {
		setSourcePrices(prices);
		setDrafts(prices);
	}

	const configured = useMemo(
		() => new Set(drafts.map((price) => price.model.trim().toLowerCase())),
		[drafts],
	);
	const availableModels = usedModels.filter((model) => !configured.has(model.trim().toLowerCase()));

	function update(index: number, field: keyof Omit<ModelPrice, "model">, value: string) {
		const parsed = Number(value);
		setDrafts((current) => current.map((price, rowIndex) => rowIndex === index
			? { ...price, [field]: Number.isFinite(parsed) ? parsed : 0 }
			: price));
	}

	function addModel() {
		const model = newModel.trim();
		if (!model || configured.has(model.toLowerCase())) return;
		setDrafts((current) => [...current, {
			model,
			input: 0,
			output: 0,
			cacheRead: 0,
			cacheWrite: 0,
			multiplier: 1,
		}]);
		setNewModel("");
	}

	return (
		<Card aria-labelledby="pricing-title">
		<CardHeader className="border-b max-sm:grid-cols-1">
			<CardTitle id="pricing-title">模型价格</CardTitle>
			<CardAction className="pricing-header-actions max-sm:col-start-1 max-sm:row-auto max-sm:mt-2 max-sm:w-full max-sm:justify-self-stretch">
				<Button disabled={syncing || saving} onClick={onSync} type="button" variant="outline">
					{syncing ? <Spinner /> : <RefreshCwIcon />}
					{syncing ? "获取中…" : "从 Models.dev 批量获取"}
				</Button>
				<Button disabled={saving || syncing} onClick={() => onSave(drafts)} type="button">
					{saving ? <Spinner /> : null}
					{saving ? "保存中…" : "保存价格"}
				</Button>
			</CardAction>
		</CardHeader>

		<CardContent className="grid gap-4">
		{error ? <Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert> : null}
		{loading ? <div className="center-state"><Spinner />读取模型价格…</div> : null}

		{!loading ? (
			<>
				<div className="pricing-add-row">
					<Field>
						<FieldLabel htmlFor="pricing-new-model">新增模型</FieldLabel>
						<Input
							id="pricing-new-model"
							list="pricing-used-models"
							onChange={(event) => setNewModel(event.target.value)}
							onKeyDown={(event) => {
								if (event.key === "Enter") {
									event.preventDefault();
									addModel();
								}
							}}
							placeholder={availableModels[0] ?? "输入模型 ID"}
							className="px-[.7rem] py-0 text-[.72rem]"
							value={newModel}
						/>
						<datalist id="pricing-used-models">
							{availableModels.map((model) => <option key={model} value={model} />)}
						</datalist>
					</Field>
					<Button disabled={!newModel.trim()} onClick={addModel} type="button" variant="outline"><PlusIcon />添加</Button>
				</div>

				{drafts.length > 0 ? (
					<ScrollArea className="rounded-[.62rem] border" scrollbars="horizontal">
						<Table className="pricing-table min-w-[58rem] table-fixed [&_th]:p-[.55rem] [&_td]:p-[.55rem] [&_th]:text-[.67rem] [&_td]:text-[.67rem] [&_th:first-child]:w-[22%] [&_th:last-child]:w-18">
							<TableHeader>
								<TableRow>
									<TableHead>模型</TableHead>
									<TableHead>普通输入</TableHead>
									<TableHead>输出</TableHead>
									<TableHead>缓存读取</TableHead>
									<TableHead>缓存写入</TableHead>
									<TableHead>倍率</TableHead>
									<TableHead><span className="sr-only">操作</span></TableHead>
								</TableRow>
							</TableHeader>
							<TableBody>
								{drafts.map((price, index) => (
									<TableRow key={price.model}>
										<TableCell data-label="模型"><code>{price.model}</code></TableCell>
										<PriceInput label="普通输入" onChange={(value) => update(index, "input", value)} value={price.input} />
										<PriceInput label="输出" onChange={(value) => update(index, "output", value)} value={price.output} />
										<PriceInput label="缓存读取" onChange={(value) => update(index, "cacheRead", value)} value={price.cacheRead} />
										<PriceInput label="缓存写入" onChange={(value) => update(index, "cacheWrite", value)} value={price.cacheWrite} />
										<PriceInput label="倍率" onChange={(value) => update(index, "multiplier", value)} value={price.multiplier} />
										<TableCell data-label="操作">
											<Button aria-label={`删除 ${price.model} 的价格`} onClick={() => setDrafts((current) => current.filter((_, rowIndex) => rowIndex !== index))} size="sm" type="button" variant="destructive">
												删除
											</Button>
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					</ScrollArea>
				) : (
					<Empty className="min-h-32 border">
						<EmptyHeader>
							<EmptyMedia variant="icon"><CircleDollarSignIcon /></EmptyMedia>
							<EmptyTitle>尚未配置模型价格</EmptyTitle>
							<EmptyDescription>可从已使用模型中添加，或使用 Models.dev 批量匹配。</EmptyDescription>
						</EmptyHeader>
					</Empty>
				)}
				<footer className="pricing-note">
					<span>金额单位：USD / 1M Token</span>
					<Button asChild size="sm" variant="link"><a href="https://models.dev/api.json" rel="noreferrer" target="_blank">查看数据源<ExternalLinkIcon /></a></Button>
				</footer>
			</>
		) : null}
		</CardContent>
	</Card>
	);
}

function PriceInput({ label, onChange, value }: { label: string; onChange: (value: string) => void; value: number }) {
	return (
		<TableCell data-label={label}>
			<Input aria-label={label} className="px-[.45rem] py-0 text-[.72rem] tabular-nums" min="0" onChange={(event) => onChange(event.target.value)} step="0.0001" type="number" value={value} />
		</TableCell>
	);
}
