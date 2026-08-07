// @ts-nocheck
/**
 * 纯函数单元测试：#3832 相关笔记提取 extractRelatedNotes。
 * 覆盖：空图、center 不存在、自环、双向边去重、悬空链接、排序。
 */
import { extractRelatedNotes } from '../../utils/relatedNotes';

const NODE = (id, title, over = {}) => ({ id, title, tags: [], in_degree: 0, out_degree: 0, ...over });

describe('extractRelatedNotes (#3832)', () => {
  it('空图（无 nodes/无 edges）返回空数组', () => {
    expect(extractRelatedNotes({ nodes: [], edges: [] }, 'n1')).toEqual([]);
    expect(extractRelatedNotes({ nodes: [], edges: [], note_count: 0, edge_count: 0, dangling_link_count: 0 }, 'n1')).toEqual([]);
  });

  it('center 不存在于 nodes 时返回空数组', () => {
    const graph = {
      nodes: [NODE('n1', '甲'), NODE('n2', '乙')],
      edges: [{ source: 'n1', target: 'n2' }],
    };
    expect(extractRelatedNotes(graph, 'ghost')).toEqual([]);
    expect(extractRelatedNotes(graph, '')).toEqual([]);
  });

  it('自环（source===target===center）不计入邻居', () => {
    const graph = {
      nodes: [NODE('n1', '甲'), NODE('n2', '乙')],
      edges: [
        { source: 'n1', target: 'n1' }, // 自环
        { source: 'n1', target: 'n2' },
      ],
    };
    const result = extractRelatedNotes(graph, 'n1');
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('n2');
    expect(result[0].linkCount).toBe(1);
  });

  it('双向边（center→A 与 A→center）去重为同一邻居，linkCount 累加', () => {
    const graph = {
      nodes: [NODE('n1', '甲'), NODE('n2', '乙')],
      edges: [
        { source: 'n1', target: 'n2' },
        { source: 'n2', target: 'n1' },
      ],
    };
    const result = extractRelatedNotes(graph, 'n1');
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('n2');
    expect(result[0].linkCount).toBe(2);
  });

  it('悬空链接（邻居不在 nodes 中）被跳过', () => {
    const graph = {
      nodes: [NODE('n1', '甲')],
      edges: [{ source: 'n1', target: 'ghost' }],
    };
    expect(extractRelatedNotes(graph, 'n1')).toEqual([]);
  });

  it('按 (in_degree + out_degree) 降序排列', () => {
    const graph = {
      nodes: [
        NODE('n1', '甲'),
        NODE('low', '低连接', { in_degree: 0, out_degree: 1 }),
        NODE('high', '高连接', { in_degree: 5, out_degree: 3 }),
        NODE('mid', '中连接', { in_degree: 2, out_degree: 1 }),
      ],
      edges: [
        { source: 'n1', target: 'low' },
        { source: 'n1', target: 'high' },
        { source: 'n1', target: 'mid' },
      ],
    };
    const result = extractRelatedNotes(graph, 'n1');
    expect(result.map(r => r.id)).toEqual(['high', 'mid', 'low']);
    expect(result[0]).toMatchObject({ in_degree: 5, out_degree: 3 });
  });

  it('邻居同时关联中心与其它笔记、且双向时仍只出现一次', () => {
    const graph = {
      nodes: [NODE('n1', '甲'), NODE('n2', '乙'), NODE('n3', '丙')],
      edges: [
        { source: 'n1', target: 'n2' },
        { source: 'n2', target: 'n1' },
        { source: 'n2', target: 'n3' }, // n2 与其它笔记的边不影响去重
      ],
    };
    const result = extractRelatedNotes(graph, 'n1');
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({ id: 'n2', linkCount: 2 });
  });
});
