package dev.evvie.waylandcraft.render;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.math.Axis;

import dev.evvie.waylandcraft.bridge.WLCToplevel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.world.phys.Vec3;

public class WindowInItemFrameRenderer {
	
	public void render(WLCToplevel toplevel, PoseStack poseStack, SubmitNodeCollector collector) {
		if(toplevel.framebuffer == null) return;
		
		poseStack.pushPose();
		poseStack.translate(1.0f, 1.0f, -0.01f);
		poseStack.scale(2.0f, 2.0f, 1.0f);
		poseStack.rotateAround(Axis.ZP.rotationDegrees(180), 0, 0, 0);
		
		int width = toplevel.geometry.width();
		int height = toplevel.geometry.height();
		int resolution;
		
		if(width > height) {
			resolution = width;
		}
		else {
			resolution = height;
		}
		
		float scale = 1.0f / resolution;
		poseStack.scale(scale, scale, 1.0f);
		
		int x = -toplevel.framebuffer.getXOff() - toplevel.geometry.x();
		int y = -toplevel.framebuffer.getYOff() - toplevel.geometry.y();
		int w = toplevel.framebuffer.getWidth();
		int h = toplevel.framebuffer.getHeight();
		
		x += resolution / 2 - width / 2;
		y += resolution / 2 - height / 2;
		
		Vec3 tl = new Vec3(x, y, 0);
		Vec3 bl = new Vec3(x, y + h, 0);
		Vec3 br = new Vec3(x + w, y + h, 0);
		Vec3 tr = new Vec3(x + w, y, 0);
		
		RenderUtils.renderFramebuffer(toplevel.framebuffer, poseStack, collector, true, false, tl, bl, br, tr);
		
		poseStack.popPose();
	}
	
}
