#ifndef ARCHIE_LUMEN_DESIGN_H
#define ARCHIE_LUMEN_DESIGN_H
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
// XaviCottier source design, Rust-owned decisions. Qt is an upload/lifecycle host.
#ifdef __cplusplus
extern "C" {
#endif
typedef struct DesignVec3 { float x,y,z; } DesignVec3;
typedef struct DesignBody { DesignVec3 position,velocity,anchor; uint32_t fixed; } DesignBody;
typedef struct DesignMotion { float radius,spring,damping,reach,reaction; } DesignMotion;
// Static null-terminated UTF-8 {schema,defaults,presets:[{id,name,description,settings}]}.
const char *design_presets_json(void);
// Style IDs: 0 original,1 prism,2 obsidian,3 titanium,4 hologram,5 crystal,
// 6 circuit,7 photon,8 ceramic. Invalid arguments return 0/false, never unwind.
float design_height(uint32_t style,float u,float v,float frequency);
// Linear RGBA8 albedo (source GPU bake), linear RGB tangent normal+A roughness.
// Each writable nonoverlapping output buffer has >=buffer_len bytes; size in 1..4096.
bool design_surface(uint32_t style,uint32_t size,float frequency,uint8_t *albedo,uint8_t *surface,size_t buffer_len);
// Result bit1=positions changed, bit2=simulation active. Pointer null means no focus.
// Bodies writable, all other pointers readable; nonoverlapping, aligned and valid.
uint32_t design_step(DesignBody *bodies,size_t count,const DesignVec3 *pointer,float dt,const DesignMotion *motion);
// Query: vertices=null,indices=null,capacities=0. Counts are vertices/indices.
// Fill: vertex_capacity is FLOAT count (8*vertex_count), index_capacity is u32 count.
// Vertex layout: position3,normal3,uv2. Style0 and style8 return false: original mesh retained.
bool design_mesh(uint32_t style,bool folder,float *vertices,size_t vertex_capacity,uint32_t *indices,size_t index_capacity,size_t *out_vertex_count,size_t *out_index_count);
// Quality IDs: 0 original/quality,1 balanced,2 performance.
uint32_t design_texture_size(uint32_t max_resolution,uint32_t max_capability,uint32_t quality,float projected_size,uint32_t current);
bool design_hybrid(uint32_t style,float depth,float emissive,float contrast,float *output,size_t len);
typedef struct DesignShellNode { DesignVec3 position,color; uint32_t branch; } DesignShellNode;
typedef struct DesignShellInput {
    uint32_t kind; const DesignShellNode *nodes; size_t count;
    float radius,density,scale; DesignVec3 warm,cool,tint;
} DesignShellInput;
// Kinds:0 fragments,1 veil,2 filaments,3 aurora. Observed nodes <=420.
// Query null/zero buffers first. Vertex capacity counts floats, stride10 pos/normal/RGBA.
// Caller provides distinct aligned input/output allocations; Rust retains nothing.
bool design_shell(const DesignShellInput *input,float *vertices,size_t vertex_capacity,
    uint32_t *indices,size_t index_capacity,size_t *vertex_count,size_t *index_count,float *radius);
// Content sphere (Xavi's desktop): read-only, bounded listing of a folder laid out by
// field.js buildField ("regions"). Writes the scene JSON LumenField::accept reads into
// out and returns its length, or 0 if the folder is unreadable or it does not fit.
typedef struct DesignContentOptions {
    uint32_t limit, files_per_folder;
    float category_pull, hierarchy_pull, radial_layers, size_influence, scale;
} DesignContentOptions;
size_t design_content_scene(const char *root, size_t root_len, const DesignContentOptions *options,
    int64_t generated_at_ms, char *out, size_t capacity);
#ifdef __cplusplus
}
#endif
#endif
