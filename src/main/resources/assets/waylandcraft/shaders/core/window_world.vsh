#version 150

layout(std140) uniform window_mvp {
	mat4 mvp;
};

in vec3 Position;
in vec2 UV0;

out vec2 texCoord0;

void main() {
	gl_Position = mvp * vec4(Position, 1.0);
	texCoord0 = UV0;
}
