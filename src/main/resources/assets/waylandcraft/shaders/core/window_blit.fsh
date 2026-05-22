#version 150
#moj_import <minecraft:projection.glsl>
#moj_import <minecraft:dynamictransforms.glsl>

uniform sampler2D Sampler0;

in vec2 texCoord0;
in vec4 vertexColor;

out vec4 fragColor;

void main() {
	vec4 color = texture(Sampler0, texCoord0);
	// Undo framebuffer alpha premultiplication. A fully transparent texel has
	// alpha 0; dividing by it yields NaN (renders as garbage), so guard it.
	color = color.a == 0.0 ? vec4(0.0) : vec4(color.rgb / color.a, color.a);
	color *= vertexColor;
	fragColor = color * ColorModulator;
}
