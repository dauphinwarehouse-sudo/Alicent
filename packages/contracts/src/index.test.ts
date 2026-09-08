import { expect, it } from 'vitest';
import { needsApproval, type ToolDefinition } from './index';
const tool: ToolDefinition = { name:'read_document',description:'Read',input_schema:{type:'object'},output_schema:{type:'object'},risk_level:'read',requires_confirmation:false,timeout_ms:1000,permission_scope:'document' };
it('read permission does not itself require confirmation', () => expect(needsApproval(tool,false)).toBe(false));
it('destructive and external operations always require confirmation', () => {
  for (const risk_level of ['destructive','external'] as const) expect(needsApproval({...tool,risk_level},true)).toBe(true);
});
it('automatic reversible writes never override explicit confirmation', () => {
  expect(needsApproval({...tool,risk_level:'write_reversible'},false)).toBe(true);
  expect(needsApproval({...tool,risk_level:'write_reversible'},true)).toBe(false);
  expect(needsApproval({...tool,requires_confirmation:true},true)).toBe(true);
});
