package dev.evvie.waylandcraft;

import org.joml.Vector2f;
import org.joml.Vector3f;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.PoseStack.Pose;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.math.Axis;

import dev.evvie.waylandcraft.bridge.WLCToplevel;
import dev.evvie.waylandcraft.bridge.WaylandCraftBridge.Size;
import dev.evvie.waylandcraft.item.WindowItem;
import dev.evvie.waylandcraft.mixin.IItemInHandRendererMixin;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.HumanoidArm;
import net.minecraft.world.item.ItemStack;

public class WindowInHandRenderer {
	
	public void render(PoseStack poseStack, MultiBufferSource multiBufferSource, float attack, float handHeight, int light, HumanoidArm humanoidArm, ItemStack itemStack) {
		poseStack.pushPose();
		
		float h = humanoidArm == HumanoidArm.RIGHT ? 1.0f : -1.0f;
		poseStack.translate(h * 0.125f, -0.125f, 0.0f);
		
		if (!Minecraft.getInstance().player.isInvisible()) {
			poseStack.pushPose();
			poseStack.mulPose(Axis.ZP.rotationDegrees(h * 10.0f));
			renderPlayerArm(poseStack, multiBufferSource, light, handHeight, attack, humanoidArm);
			poseStack.popPose();
		}
		
		poseStack.translate(h * 0.8f, handHeight * -0.6f - 0.3, -0.85f);
		
		float sattack = Mth.sqrt(attack);
		float osci = Mth.sin(sattack * (float) Math.PI);
		float dx = -0.6f * osci;
		float dy = 0.55f * Mth.sin(sattack * (float) (Math.PI * 2));
		float dz = -0.6f * Mth.sin(attack * (float) Math.PI);
		poseStack.translate(h * dx, dy - 0.3f * osci, dz);
		poseStack.mulPose(Axis.XP.rotationDegrees(osci * -45.0f));
		poseStack.mulPose(Axis.YP.rotationDegrees(h * osci * -30.0f));
		
		renderWindow(poseStack, multiBufferSource, h, light, itemStack);
		
		poseStack.popPose();
	}
	
	public void renderWindow(PoseStack poseStack, MultiBufferSource source, float sideMult, int light, ItemStack itemStack) {
		WLCToplevel toplevel = WindowItem.getToplevel(itemStack);
		if(toplevel == null) return;
		if(toplevel.framebuffer == null) return;
		
		int width = toplevel.geometry.width();
		int height = toplevel.geometry.height();
		
		Size outputSize = WaylandCraft.instance.bridge.getOutputSize();
		float scale = 0.82f / Math.max(outputSize.width(), outputSize.height());
		
		poseStack.scale(scale, scale, 1);
		poseStack.translate(-width / 2 * sideMult, height / 2, 0);
		poseStack.scale(width, height, 1);
		poseStack.translate(-0.5, -0.5, 0);
		
		Pose pose = poseStack.last();
		VertexConsumer buffer = source.getBuffer(WaylandCraftRenderTypes.window(toplevel.framebuffer.getTexture()));
		Vector3f pos1 = pose.pose().transformPosition(0, 1, 0, new Vector3f());
		Vector3f pos2 = pose.pose().transformPosition(0, 0, 0, new Vector3f());
		Vector3f pos3 = pose.pose().transformPosition(1, 0, 0, new Vector3f());
		Vector3f pos4 = pose.pose().transformPosition(1, 1, 0, new Vector3f());
		
		Vector2f uv1 = new Vector2f(0, 0);
		Vector2f uv2 = new Vector2f(0, 1);
		Vector2f uv3 = new Vector2f(1, 1);
		Vector2f uv4 = new Vector2f(1, 0);
		
		Vector3f normal = pose.transformNormal(0, 0, 1, new Vector3f());
		
		int overlayCoords = OverlayTexture.NO_OVERLAY;
		light = LightTexture.FULL_BRIGHT;
		
		// Front quad
		buffer.vertex(/* pos */ pos1.x, pos1.y, pos1.z, /* color */ 1, 1, 1, 1, /* uv */ uv1.x, uv1.y, /* overlay */ overlayCoords, /* uv2 */ light, /* normal */ normal.x, normal.y, normal.z);
		buffer.vertex(/* pos */ pos2.x, pos2.y, pos2.z, /* color */ 1, 1, 1, 1, /* uv */ uv2.x, uv2.y, /* overlay */ overlayCoords, /* uv2 */ light, /* normal */ normal.x, normal.y, normal.z);
		buffer.vertex(/* pos */ pos3.x, pos3.y, pos3.z, /* color */ 1, 1, 1, 1, /* uv */ uv3.x, uv3.y, /* overlay */ overlayCoords, /* uv2 */ light, /* normal */ normal.x, normal.y, normal.z);
		buffer.vertex(/* pos */ pos4.x, pos4.y, pos4.z, /* color */ 1, 1, 1, 1, /* uv */ uv4.x, uv4.y, /* overlay */ overlayCoords, /* uv2 */ light, /* normal */ normal.x, normal.y, normal.z);
	}
	
	public void renderPlayerArm(PoseStack poseStack, MultiBufferSource multiBufferSource, int light, float handHeight, float attack, HumanoidArm humanoidArm) {
		((IItemInHandRendererMixin) Minecraft.getInstance().getEntityRenderDispatcher().getItemInHandRenderer()).invokeRenderPlayerArm(poseStack, multiBufferSource, light, handHeight, attack, humanoidArm);
	}
	
}
