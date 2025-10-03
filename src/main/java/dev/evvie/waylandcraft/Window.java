package dev.evvie.waylandcraft;

import org.joml.Matrix4f;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.Tesselator;
import com.mojang.blaze3d.vertex.VertexFormat;

import dev.evvie.waylandcraft.bridge.WLCSurface;
import dev.evvie.waylandcraft.bridge.WLCToplevel;
import net.fabricmc.fabric.api.client.rendering.v1.WorldRenderContext;
import net.minecraft.client.Camera;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.world.phys.Vec3;

public class Window {
	
	private static final float PIXEL_SCALE = 1.0f / 500;
	
	public final WLCToplevel toplevel;
	
	// World position of window
	public Vec3 pivot = new Vec3(-250, 65, -500);
	
	// Window facing direction normal
	private Vec3 normal = new Vec3(0, 0, 1);
	
	// Window orientation downwards vector, has to be orthogonal to `normal` and normalized
	private Vec3 down = new Vec3(0, -1, 0);
	
	private int width;
	private int height;
	
	public Window(WLCToplevel toplevel) {
		this.toplevel = toplevel;
	}
	
	public boolean isAlive() {
		return toplevel.isAlive();
	}
	
	private Vec3 right() {
		return normal.cross(down);
	}
	
	private void updateGeometry() {
		BufferTexture buf = toplevel.getSurfaceTree().getBuffer();
		if(buf == null) {
			width = 0;
			height = 0;
		}
		else {
			width = buf.width;
			height = buf.height;
		}
	}
	
	public void render(WorldRenderContext ctx) {
		updateGeometry();
		
//		normal = new Vec3(ctx.camera().getLookVector()).reverse();
//		down = new Vec3(ctx.camera().getUpVector()).reverse();
		
		int depth = 0;
		for(WLCSurface surface = toplevel.getSurfaceTree(); surface != null; surface = surface.getNextChild()) {
			renderSurface(ctx, surface, depth);
			depth++;
		}
	}
	
	private void renderSurface(WorldRenderContext ctx, WLCSurface surface, int depth) {
		Vec3 localX = right().scale(PIXEL_SCALE);
		Vec3 localY = down.scale(PIXEL_SCALE);
		Vec3 origin = pivot.add(localX.scale(-width/2)).add(localY.scale(-height/2));
		origin = origin.add(localX.scale(surface.xSubpos)).add(localY.scale(surface.ySubpos));
		origin = origin.add(normal.scale(depth * 0.0001));
		
		BufferTexture buf = surface.getBuffer();
		if(buf == null) return;
		
		Vec3 tl = origin;
		Vec3 bl = origin.add(localY.scale(buf.height));
		Vec3 br = bl.add(localX.scale(buf.width));
		Vec3 tr = tl.add(localX.scale(buf.width));
		
		Camera camera = ctx.camera();
		Tesselator tesselator = Tesselator.getInstance();
		BufferBuilder buffer = tesselator.getBuilder();
		PoseStack matrixStack = new PoseStack();
		matrixStack.translate(-camera.getPosition().x, -camera.getPosition().y, -camera.getPosition().z);
		Matrix4f mat = matrixStack.last().pose();
		
		buffer.begin(VertexFormat.Mode.QUADS, DefaultVertexFormat.POSITION_COLOR_TEX);
		buffer.vertex(mat, (float) tl.x, (float) tl.y, (float) tl.z).color(1.0f, 1.0f, 1.0f, 1.0f).uv(0, 0).endVertex();
		buffer.vertex(mat, (float) bl.x, (float) bl.y, (float) bl.z).color(1.0f, 1.0f, 1.0f, 1.0f).uv(0, 1).endVertex();
		buffer.vertex(mat, (float) br.x, (float) br.y, (float) br.z).color(1.0f, 1.0f, 1.0f, 1.0f).uv(1, 1).endVertex();
		buffer.vertex(mat, (float) tr.x, (float) tr.y, (float) tr.z).color(1.0f, 1.0f, 1.0f, 1.0f).uv(1, 0).endVertex();
		
		RenderSystem.setShader(GameRenderer::getPositionColorTexShader);
		RenderSystem.setShaderTexture(0, buf.getId());
		RenderSystem.setShaderColor(1f, 1f, 1f, 1f);
		tesselator.end();
	}
	
}
