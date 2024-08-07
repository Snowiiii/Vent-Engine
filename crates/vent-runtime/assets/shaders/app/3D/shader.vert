#version 450 core

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec2 in_tex_coord;
layout(location = 2) in vec3 in_normal;

layout(push_constant) uniform PushConsts {
    vec3 view_position;
    mat4 proj_view_trans;
} camera;

layout(location = 0) out vec2 tex_coord;
layout(location = 1) out vec3 normal;
layout(location = 2) out vec3 world_position;
layout(location = 3) out vec4 position;
layout(location = 4) out vec3 view_position;

void main() {
    tex_coord = in_tex_coord;
    normal = in_normal;
    world_position = in_position;
    position = camera.proj_view_trans * vec4(in_position, 1.0);
    view_position = camera.view_position;

    gl_Position = position;
}