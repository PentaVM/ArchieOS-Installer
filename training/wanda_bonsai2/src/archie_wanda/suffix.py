"""Differentiable final full-attention block of the exact frozen Bonsai2 base."""
import torch
from torch import nn
from .contracts import AdmissionError, BASE_SHA
from .model import read_tensor_rows
from .ptq import FrozenPTQLinear


def rms_norm(value, weight, epsilon):
    return value * torch.rsqrt(value.square().mean(dim=-1,keepdim=True)+epsilon) * weight


def split_query_gate(value, heads, head_dim):
    shaped=value.reshape(value.shape[0],heads,2,head_dim)
    return shaped[:,:,0,:], shaped[:,:,1,:]


def text_rope(value, positions, dimensions, base):
    if dimensions % 2 or dimensions>value.shape[-1] or positions.shape!=(value.shape[0],):
        raise AdmissionError('unsupported text RoPE layout')
    frequency=base**(-torch.arange(0,dimensions,2,device=value.device,dtype=value.dtype)/dimensions)
    angle=positions.to(value.dtype)[:,None]*frequency[None,:]
    cosine,sine=angle.cos()[:,None,:],angle.sin()[:,None,:]
    left,right=value[...,:dimensions//2],value[...,dimensions//2:dimensions]
    return torch.cat((left*cosine-right*sine,left*sine+right*cosine,value[...,dimensions:]),dim=-1)


class LowRankDelta(nn.Module):
    def __init__(self, input_size, output_size, rank=8, alpha=16.):
        super().__init__()
        self.a=nn.Parameter(torch.randn(rank,input_size)*.001)
        self.b=nn.Parameter(torch.zeros(output_size,rank))
        self.scale=alpha/rank

    def forward(self, original_activation):
        return ((original_activation @ self.a.T) @ self.b.T)*self.scale


TARGETS = {'q': ('attn_q', 4096, 8192), 'k': ('attn_k', 4096, 1024),
           'v': ('attn_v', 4096, 1024), 'o': ('attn_output', 4096, 4096)}


class BonsaiSuffix(nn.Module):
    def __init__(self, model, config, targets=('q', 'v')):
        super().__init__()
        if not targets or set(targets)-set(TARGETS) or len(set(targets))!=len(targets):
            raise AdmissionError('unsupported LoRA target set')
        self.targets=tuple(sorted(targets,key='qkvo'.index))
        if model.sha256!=BASE_SHA:
            raise AdmissionError('suffix requires the exact approved base')
        self.model,self.config=model,config
        fields=model.fields
        expected={'qwen35.block_count':32,'qwen35.embedding_length':4096,
                  'qwen35.attention.head_count':16,'qwen35.attention.head_count_kv':4,
                  'qwen35.attention.key_length':256,'qwen35.full_attention_interval':4}
        if any(fields.get(k)!=v for k,v in expected.items()):
            raise AdmissionError('unsupported exact-model geometry')
        self.epsilon=fields['qwen35.attention.layer_norm_rms_epsilon']
        self.rope_dimensions=fields['qwen35.rope.dimension_count']
        self.rope_base=fields['qwen35.rope.freq_base']
        self.linear=nn.ModuleDict({key:FrozenPTQLinear(model,'blk.31.'+key+'.weight',config.row_chunk)
                                  for key in ['attn_q','attn_k','attn_v','attn_output','ffn_gate','ffn_up','ffn_down']})
        for key,name in [('input_norm','blk.31.attn_norm.weight'),
                         ('q_norm','blk.31.attn_q_norm.weight'),('k_norm','blk.31.attn_k_norm.weight'),
                         ('post_norm','blk.31.post_attention_norm.weight'),('output_norm','output_norm.weight')]:
            self.register_buffer(key,torch.from_numpy(read_tensor_rows(model,name,0,1).reshape(-1)))
        for target in self.targets:
            _, columns, rows = TARGETS[target]
            setattr(self, target+'_delta', LowRankDelta(columns,rows,config.rank,config.alpha))

    def delta(self, target, activation):
        layer=getattr(self, target+'_delta', None)
        return 0. if layer is None else layer(activation)

    def make_resident(self, device):
        for layer in self.linear.values(): layer.make_resident(device)
        return self

    def forward(self, hidden, positions=None, sequence_layout=None):
        count=hidden.shape[0]
        if hidden.ndim!=2 or hidden.shape[1]!=4096 or not 0<count<=self.config.sequence_length:
            raise AdmissionError('full suffix expects complete bounded [tokens,4096] input')
        if positions is None: positions=torch.arange(count,device=hidden.device)
        if not torch.equal(positions,torch.arange(count,device=hidden.device)):
            raise AdmissionError('initial suffix requires a complete sequence from position0')
        if sequence_layout is not None and (sequence_layout.shape!=(count,) or torch.any(sequence_layout!=0).item()):
            raise AdmissionError('initial suffix supports one fresh sequence only')
        normalized=rms_norm(hidden,self.input_norm,self.epsilon)
        query,gate=split_query_gate(self.linear['attn_q'](normalized)+self.delta('q',normalized),16,256)
        key=(self.linear['attn_k'](normalized)+self.delta('k',normalized)).reshape(count,4,256)
        value=(self.linear['attn_v'](normalized)+self.delta('v',normalized)).reshape(count,4,256)
        query=rms_norm(query,self.q_norm,self.epsilon)
        key=rms_norm(key,self.k_norm,self.epsilon)
        query=text_rope(query,positions,self.rope_dimensions,self.rope_base).transpose(0,1)
        key=text_rope(key,positions,self.rope_dimensions,self.rope_base).repeat_interleave(4,dim=1).transpose(0,1)
        value=value.repeat_interleave(4,dim=1).transpose(0,1)
        scores=(query @ key.transpose(-1,-2))/16.
        causal=torch.ones((count,count),device=hidden.device,dtype=torch.bool).triu(1)
        scores=scores.masked_fill(causal,float('-inf'))
        attended=(torch.softmax(scores,dim=-1) @ value).transpose(0,1).reshape(count,4096)
        attended=attended*torch.sigmoid(gate.reshape(count,4096))
        residual=hidden+self.linear['attn_output'](attended)+self.delta('o',attended)
        post=rms_norm(residual,self.post_norm,self.epsilon)
        feedforward=torch.nn.functional.silu(self.linear['ffn_gate'](post))*self.linear['ffn_up'](post)
        result=residual+self.linear['ffn_down'](feedforward)
        return rms_norm(result,self.output_norm,self.epsilon)
