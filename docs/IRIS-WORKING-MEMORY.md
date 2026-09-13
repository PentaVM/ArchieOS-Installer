# IRIS working memory

Working Scene Memory keeps bounded reflex entries, expiring semantic episodes and a
fixed-capacity delta ring. TTL and capacity eviction are deterministic. Durable output is
only a semantic promotion candidate; the memory admission firewall still decides whether
it may persist. Pixels, audio and transcripts have no durable representation here.
