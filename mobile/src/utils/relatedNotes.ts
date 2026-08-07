/**
 * 相关笔记 / 局部图谱（local graph）提取 —— #3832 mobile 部分。
 *
 * 纯函数模块：从后端返回的完整知识图谱（KnowledgeGraph JSON，见
 * src/knowledge_graph.rs 的 GraphNode/GraphEdge/KnowledgeGraph）中，
 * 在客户端提取某笔记的 1 跳邻居（相关笔记）。
 *
 * 本文件不依赖 react-native，方便在 node 环境下做单元测试。
 */

export interface GraphNode {
  id: string;
  title: string;
  tags?: string[];
  in_degree?: number;
  out_degree?: number;
}

export interface GraphEdge {
  source: string;
  target: string;
  label?: string;
  kind?: string;
}

export interface KnowledgeGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  note_count?: number;
  edge_count?: number;
  dangling_link_count?: number;
}

export interface RelatedNote {
  id: string;
  title: string;
  in_degree: number;
  out_degree: number;
  tags: string[];
  /** 该邻居与中心笔记之间的边数（双向/多条边合并计数，自环不计） */
  linkCount: number;
}

/**
 * 从完整图谱中提取 centerId 的 1 跳邻居，按 (in_degree + out_degree)
 * 降序排列（连接越多的笔记越靠前）。
 *
 * 行为约定：
 * - 空图 / centerId 为空 / center 不在 nodes 中 → 返回 []。
 * - 自环（source === target === centerId）不计入邻居。
 * - 双向边（center→A 与 A→center）去重为同一个邻居，linkCount 累加。
 * - 边指向的节点不在 nodes 中（悬空链接）→ 跳过该邻居。
 */
export function extractRelatedNotes(graph: KnowledgeGraph, centerId: string): RelatedNote[] {
  const nodes = Array.isArray(graph?.nodes) ? graph.nodes : [];
  const edges = Array.isArray(graph?.edges) ? graph.edges : [];
  if (!centerId || nodes.length === 0 || edges.length === 0) return [];

  const nodeById = new Map<string, GraphNode>();
  for (const n of nodes) {
    if (n && typeof n.id === 'string') nodeById.set(n.id, n);
  }
  if (!nodeById.has(centerId)) return [];

  // 邻居 id → 与中心笔记相连的边数（去重计数）
  const neighbors = new Map<string, { count: number }>();
  for (const e of edges) {
    if (!e || typeof e.source !== 'string' || typeof e.target !== 'string') continue;
    if (e.source === centerId && e.target === centerId) continue; // 自环
    let other: string | null = null;
    if (e.source === centerId) other = e.target;
    else if (e.target === centerId) other = e.source;
    if (other === null) continue;
    const entry = neighbors.get(other) ?? { count: 0 };
    entry.count += 1;
    neighbors.set(other, entry);
  }

  const out: RelatedNote[] = [];
  for (const [id, { count }] of neighbors) {
    const node = nodeById.get(id);
    if (!node) continue; // 悬空链接：邻居节点不存在于 nodes
    out.push({
      id: node.id,
      title: node.title ?? '',
      in_degree: node.in_degree ?? 0,
      out_degree: node.out_degree ?? 0,
      tags: Array.isArray(node.tags) ? node.tags : [],
      linkCount: count,
    });
  }

  out.sort((a, b) => b.in_degree + b.out_degree - (a.in_degree + a.out_degree));
  return out;
}
