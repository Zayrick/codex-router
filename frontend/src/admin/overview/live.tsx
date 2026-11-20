import { useEffect, useMemo, useState } from "react";
import { ResponsiveContainer, Sankey, type SankeyNodeProps, type SankeyLinkProps } from "recharts";

import type { LiveConnection } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { FootChip } from "@/admin/overview/activity";
import { StatusDot, Tag } from "@/components/app/badges";
import { SectionCard } from "@/components/app/section-card";
import { identityTypeLabel, seriesColor, VIZ } from "@/components/app/viz";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatCount, formatSpan } from "@/lib/format";

/** Column of a flow node: who sent it, which pool routed it, which account serves it. */
type Column = 0 | 1 | 2;

interface FlowNode {
	name: string;
	column: Column;
	color: string;
	websocket: number;
	http: number;
	active: number;
}

interface FlowLink {
	source: number;
	target: number;
	value: number;
}

const COLUMN_TITLES = ["用户 · 凭据", "账户组", "Codex 账户"] as const;
const DIRECT_GROUP = "直接分配";
const ROW_HEIGHT = 34;
const MAX_LABEL_CHARS = 22;

function credentialLabel(connection: LiveConnection): string {
	const credential = connection.credentialName || identityTypeLabel(connection.credentialType);
	return `${connection.userName} · ${credential}`;
}

function clip(text: string): string {
	return text.length > MAX_LABEL_CHARS ? `${text.slice(0, MAX_LABEL_CHARS - 1)}…` : text;
}

/** Three-column flow (credential → group → account) weighted by connection count. */
function buildFlow(connections: LiveConnection[]): { nodes: FlowNode[]; links: FlowLink[]; rows: number } {
	const nodes: FlowNode[] = [];
	const index = new Map<string, number>();
	const links = new Map<string, FlowLink>();
	let credentials = 0;

	const node = (key: string, name: string, column: Column, color: () => string) => {
		let at = index.get(key);
		if (at === undefined) {
			at = nodes.length;
			index.set(key, at);
			nodes.push({ name, column, color: color(), websocket: 0, http: 0, active: 0 });
		}
		return at;
	};
	const link = (source: number, target: number) => {
		const key = `${source}>${target}`;
		const entry = links.get(key) ?? { source, target, value: 0 };
		entry.value += 1;
		links.set(key, entry);
	};

	for (const connection of connections) {
		const path = [
			node(`c:${connection.credentialId || connection.userId}`, credentialLabel(connection), 0, () => seriesColor(credentials++)),
			node(`g:${connection.accountGroupId}`, connection.accountGroupName || DIRECT_GROUP, 1, () => VIZ.gray),
			node(`a:${connection.codexAccountId}`, connection.codexAccountName || connection.codexAccountId, 2, () => VIZ.teal),
		];
		for (const at of path) {
			const entry = nodes[at]!;
			if (connection.transport === "websocket") entry.websocket += 1;
			else entry.http += 1;
			if (connection.active) entry.active += 1;
		}
		link(path[0]!, path[1]!);
		link(path[1]!, path[2]!);
	}
	const perColumn = [0, 0, 0];
	for (const entry of nodes) perColumn[entry.column]! += 1;
	return { nodes, links: [...links.values()], rows: Math.max(...perColumn) };
}

function nodeSummary(entry: FlowNode): string {
	const parts = [`${entry.websocket + entry.http} 个连接`];
	if (entry.websocket) parts.push(`WebSocket ${entry.websocket}`);
	if (entry.http) parts.push(`HTTP ${entry.http}`);
	if (entry.active) parts.push(`${entry.active} 个生成中`);
	return parts.join(" · ");
}

function FlowNodeShape(props: SankeyNodeProps) {
	const { x, y, width, height, payload } = props;
	const entry = payload as unknown as FlowNode;
	const total = entry.websocket + entry.http;
	// Labels sit inside the chart: right of the first two columns, left of the last.
	const right = entry.column < 2;
	const textX = right ? x + width + 8 : x - 8;
	return (
		<g>
			<rect fill={entry.color} height={Math.max(2, height)} rx={2} width={width} x={x} y={y}>
				<title>{`${entry.name}\n${nodeSummary(entry)}`}</title>
			</rect>
			<text
				dominantBaseline="middle"
				fill="var(--foreground)"
				fontSize={12}
				paintOrder="stroke"
				stroke="var(--card)"
				strokeWidth={4}
				textAnchor={right ? "start" : "end"}
				x={textX}
				y={y + height / 2}
			>
				{clip(entry.name)}
				<tspan dx={6} fill="var(--muted-foreground)" fontWeight={600}>
					{total}
				</tspan>
			</text>
		</g>
	);
}

function FlowLinkShape(props: SankeyLinkProps) {
	const { sourceX, targetX, sourceY, targetY, sourceControlX, targetControlX, linkWidth, payload } = props;
	const source = payload.source as unknown as FlowNode;
	const target = payload.target as unknown as FlowNode;
	return (
		<path
			d={`M${sourceX},${sourceY}C${sourceControlX},${sourceY} ${targetControlX},${targetY} ${targetX},${targetY}`}
			fill="none"
			stroke={source.column === 0 ? source.color : target.color}
			strokeOpacity={0.28}
			strokeWidth={Math.max(1, linkWidth)}
		>
			<title>{`${source.name} → ${target.name}\n${payload.value} 个连接`}</title>
		</path>
	);
}

/** Ticks every second while mounted, for connection ages. */
function useSecondClock(): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const timer = window.setInterval(() => setNow(Date.now()), 1_000);
		return () => window.clearInterval(timer);
	}, []);
	return now;
}

function ConnectionTable({ connections }: { connections: LiveConnection[] }) {
	const now = useSecondClock();
	return (
		<Table>
			<TableHeader>
				<TableRow>
					<TableHead>传输</TableHead>
					<TableHead>用户 · 凭据</TableHead>
					<TableHead>路由</TableHead>
					<TableHead>模型</TableHead>
					<TableHead>状态</TableHead>
					<TableHead className="text-right">请求</TableHead>
					<TableHead className="text-right">已连接</TableHead>
					<TableHead>客户端</TableHead>
				</TableRow>
			</TableHeader>
			<TableBody>
				{connections.map((connection) => (
					<TableRow key={connection.id}>
						<TableCell>
							<Tag tone={connection.transport === "websocket" ? "info" : "neutral"}>{connection.transport === "websocket" ? "WS" : "HTTP"}</Tag>
						</TableCell>
						<TableCell className="max-w-56 truncate" title={`${credentialLabel(connection)}（${identityTypeLabel(connection.credentialType)}）`}>
							{credentialLabel(connection)}
						</TableCell>
						<TableCell className="max-w-64 truncate">
							<span className="text-muted-foreground">{connection.accountGroupName || DIRECT_GROUP} → </span>
							{connection.codexAccountName || connection.codexAccountId}
						</TableCell>
						<TableCell className="font-mono text-xs">{connection.model || "—"}</TableCell>
						<TableCell>
							<StatusDot tone={connection.active ? "success" : "neutral"}>{connection.active ? "生成中" : "空闲"}</StatusDot>
						</TableCell>
						<TableCell className="text-right tabular-nums">{formatCount(connection.requests)}</TableCell>
						<TableCell className="text-right tabular-nums">{formatSpan(Math.max(0, now - connection.startedAt))}</TableCell>
						<TableCell className="max-w-40 truncate text-muted-foreground" title={connection.userAgent || undefined}>
							{connection.clientIp || "—"}
						</TableCell>
					</TableRow>
				))}
			</TableBody>
		</Table>
	);
}

/** Requests and WebSockets being proxied now, as a credential → group → account flow. */
export function LiveConnectionsCard() {
	const { live } = useAdmin();
	const connections = live.connections;
	const flow = useMemo(() => buildFlow(connections), [connections]);
	const websockets = connections.filter((connection) => connection.transport === "websocket").length;
	const active = connections.filter((connection) => connection.active).length;

	const status = live.connected ? (
		<StatusDot tone="success">实时</StatusDot>
	) : (
		<StatusDot tone="warning">{live.at === null ? "连接中…" : "已断开，正在重连"}</StatusDot>
	);

	return (
		<SectionCard actions={<span className="text-xs text-muted-foreground">{status}</span>} description="正在代理的请求与 WebSocket，以及它们被路由到的账户" title="实时连接">
			<div className="flex flex-wrap items-center gap-2">
				<FootChip label="连接" value={formatCount(connections.length)} />
				<FootChip label="WebSocket" value={formatCount(websockets)} />
				<FootChip label="HTTP" value={formatCount(connections.length - websockets)} />
				<FootChip label="生成中" value={formatCount(active)} />
			</div>
			{connections.length === 0 ? (
				<div className="flex h-24 items-center justify-center rounded-lg border border-dashed text-[12.5px] text-muted-foreground">
					{live.connected ? "当前没有活动连接。" : "等待实时数据…"}
				</div>
			) : (
				<>
					<div className="grid grid-cols-3 text-[11.5px] font-medium text-faint">
						{COLUMN_TITLES.map((title, column) => (
							<span className={column === 1 ? "text-center" : column === 2 ? "text-right" : ""} key={title}>
								{title}
							</span>
						))}
					</div>
					<div style={{ height: Math.max(140, flow.rows * ROW_HEIGHT) }}>
						<ResponsiveContainer height="100%" width="100%">
							<Sankey
								data={{ nodes: flow.nodes, links: flow.links }}
								iterations={64}
								link={FlowLinkShape}
								margin={{ top: 6, right: 4, bottom: 6, left: 4 }}
								node={FlowNodeShape}
								nodePadding={14}
								nodeWidth={8}
							/>
						</ResponsiveContainer>
					</div>
					<ConnectionTable connections={connections} />
				</>
			)}
		</SectionCard>
	);
}
