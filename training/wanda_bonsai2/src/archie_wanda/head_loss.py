"""Full-vocabulary cross entropy with bounded quantized-head replay backward.

The head is frozen; every vocabulary class contributes to normalization.
No dense head or per-chunk forward graph is retained for backward.
"""
import torch
from .contracts import AdmissionError, ParityError


class _FullVocabularyLoss(torch.autograd.Function):
    @staticmethod
    def forward(ctx, hidden, labels, head, chunk_size):
        flat=hidden.reshape(-1,hidden.shape[-1])
        labels=labels.reshape(-1)
        valid=labels!=-100
        transformed=head.transform(flat)
        log_normalizer=torch.full((flat.shape[0],),float('-inf'),dtype=flat.dtype,device=flat.device)
        target=torch.zeros_like(log_normalizer)
        for first in range(0,head.rows,chunk_size):
            weight=head.chunk(first,flat,chunk_size)
            scores=transformed @ weight.T
            log_normalizer=torch.logaddexp(log_normalizer,torch.logsumexp(scores,dim=-1))
            selected=valid & (labels>=first) & (labels<first+weight.shape[0])
            rows=selected.nonzero().flatten()
            target[rows]=scores[rows,labels[rows]-first]
        loss=(log_normalizer[valid]-target[valid]).mean()
        if not torch.isfinite(loss).item():
            raise ParityError('nonfinite full-vocabulary loss')
        ctx.head,ctx.chunk_size,ctx.original_shape=head,chunk_size,hidden.shape
        ctx.save_for_backward(hidden,labels,log_normalizer)
        return loss

    @staticmethod
    def backward(ctx, grad_loss):
        hidden,labels,log_normalizer=ctx.saved_tensors
        flat=hidden.reshape(-1,hidden.shape[-1])
        transformed=ctx.head.transform(flat)
        valid=labels!=-100
        count=valid.sum()
        dx=torch.zeros_like(flat)
        for first in range(0,ctx.head.rows,ctx.chunk_size):
            weight=ctx.head.chunk(first,flat,ctx.chunk_size)
            scores=transformed @ weight.T
            gradient=torch.exp(scores-log_normalizer[:,None])
            selected=valid & (labels>=first) & (labels<first+weight.shape[0])
            rows=selected.nonzero().flatten()
            gradient[rows,labels[rows]-first]-=1
            gradient[~valid]=0
            dx.addmm_(gradient/count,weight)
        return (ctx.head.transform_transpose(dx)*grad_loss).reshape(ctx.original_shape),None,None,None


def full_vocab_cross_entropy(hidden, labels, head, vocab_chunk=2048):
    if (hidden.ndim<2 or labels.shape!=hidden.shape[:-1] or labels.dtype!=torch.int64 or
        labels.device!=hidden.device or type(vocab_chunk) is not int or not 1<=vocab_chunk<=4096):
        raise AdmissionError('invalid head loss layout/chunk')
    valid=labels!=-100
    if not valid.any().item() or torch.any(valid & ((labels<0) | (labels>=head.rows))).item():
        raise AdmissionError('at least one valid in-vocabulary target is required')
    if not torch.isfinite(hidden).all().item():
        raise AdmissionError('nonfinite head input')
    return _FullVocabularyLoss.apply(hidden,labels,head,vocab_chunk)
