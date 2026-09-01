import { useMemo, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
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
		<section className="card pricing-card" aria-labelledby="pricing-title">
		<header className="pricing-header">
			<div>
				<h2 id="pricing-title">模型价格</h2>
				<p>按每百万 Token 的美元单价计算成本，配置会写入服务器的 <code>config.toml</code>。</p>
			</div>
			<div className="pricing-header-actions">
				<button className="button button-secondary" disabled={syncing || saving} onClick={onSync} type="button">
					{syncing ? <span className="spinner" aria-hidden="true" /> : <SyncIcon />}
					{syncing ? "获取中…" : "从 Models.dev 批量获取"}
				</button>
				<button className="button button-primary" disabled={saving || syncing} onClick={() => onSave(drafts)} type="button">
					{saving ? <span className="spinner" aria-hidden="true" /> : null}
					{saving ? "保存中…" : "保存价格"}
				</button>
			</div>
		</header>

		{error ? <div className="inline-alert error-alert pricing-alert" role="alert">{error}</div> : null}
		{loading ? <div className="center-state pricing-loading"><span className="spinner" aria-hidden="true" />读取模型价格…</div> : null}

		{!loading ? (
			<>
				<div className="pricing-add-row">
					<label>
						<span>新增模型</span>
						<input
							list="pricing-used-models"
							onChange={(event) => setNewModel(event.target.value)}
							onKeyDown={(event) => {
								if (event.key === "Enter") {
									event.preventDefault();
									addModel();
								}
							}}
							placeholder={availableModels[0] ?? "输入模型 ID"}
							value={newModel}
						/>
						<datalist id="pricing-used-models">
							{availableModels.map((model) => <option key={model} value={model} />)}
						</datalist>
					</label>
					<button className="button button-secondary" disabled={!newModel.trim()} onClick={addModel} type="button">添加</button>
				</div>

				{drafts.length > 0 ? (
					<ScrollArea className="pricing-table-wrap" scrollbars="horizontal">
						<table className="pricing-table">
							<thead>
								<tr>
									<th>模型</th>
									<th>普通输入</th>
									<th>输出</th>
									<th>缓存读取</th>
									<th>缓存写入</th>
									<th>倍率</th>
									<th><span className="sr-only">操作</span></th>
								</tr>
							</thead>
							<tbody>
								{drafts.map((price, index) => (
									<tr key={price.model}>
										<td data-label="模型"><code>{price.model}</code></td>
										<PriceInput label="普通输入" onChange={(value) => update(index, "input", value)} value={price.input} />
										<PriceInput label="输出" onChange={(value) => update(index, "output", value)} value={price.output} />
										<PriceInput label="缓存读取" onChange={(value) => update(index, "cacheRead", value)} value={price.cacheRead} />
										<PriceInput label="缓存写入" onChange={(value) => update(index, "cacheWrite", value)} value={price.cacheWrite} />
										<PriceInput label="倍率" onChange={(value) => update(index, "multiplier", value)} value={price.multiplier} />
										<td data-label="操作">
											<button aria-label={`删除 ${price.model} 的价格`} className="pricing-delete" onClick={() => setDrafts((current) => current.filter((_, rowIndex) => rowIndex !== index))} type="button">
												删除
											</button>
										</td>
									</tr>
								))}
							</tbody>
						</table>
					</ScrollArea>
				) : (
					<div className="empty-state pricing-empty">
						<strong>尚未配置模型价格</strong>
						<p>可从已使用模型中添加，或使用 Models.dev 批量匹配。</p>
					</div>
				)}
				<footer className="pricing-note">
					<span>金额单位：USD / 1M Token</span>
					<a href="https://models.dev/api.json" rel="noreferrer" target="_blank">查看数据源</a>
				</footer>
			</>
		) : null}
	</section>
	);
}

function PriceInput({ label, onChange, value }: { label: string; onChange: (value: string) => void; value: number }) {
	return (
		<td data-label={label}>
			<input aria-label={label} min="0" onChange={(event) => onChange(event.target.value)} step="0.0001" type="number" value={value} />
		</td>
	);
}

function SyncIcon() {
	return (
		<svg aria-hidden="true" className="icon" fill="none" viewBox="0 0 24 24">
			<path d="M20 7h-5V2M4 17h5v5M19 12a7 7 0 0 0-12-5l-3 3m1 2a7 7 0 0 0 12 5l3-3" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" />
		</svg>
	);
}
