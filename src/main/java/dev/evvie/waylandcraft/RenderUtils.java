package dev.evvie.waylandcraft;

import java.io.IOException;

import org.joml.Matrix4f;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.BufferUploader;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.PoseStack.Pose;
import com.mojang.blaze3d.vertex.Tesselator;
import com.mojang.blaze3d.vertex.VertexFormat;
import com.mojang.math.Axis;

import net.fabricmc.fabric.api.client.rendering.v1.CoreShaderRegistrationCallback;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.ShaderInstance;
import net.minecraft.resources.ResourceLocation;

public class RenderUtils {
	
	private static ShaderInstance CUTOUT_NO_COLOR;
	private static ShaderInstance RENDERTYPE_WINDOW;
	
	protected static void registerShaders(CoreShaderRegistrationCallback.RegistrationContext context) throws IOException {
		context.register(new ResourceLocation(WaylandCraft.MOD_ID, "cutout_no_color"), DefaultVertexFormat.POSITION_TEX, shader -> {
			CUTOUT_NO_COLOR = shader;
		});
		context.register(new ResourceLocation(WaylandCraft.MOD_ID, "rendertype_window"), DefaultVertexFormat.NEW_ENTITY, shader -> {
			RENDERTYPE_WINDOW = shader;
		});
	}
	
	public static ShaderInstance getCutoutNoColor() {
		return CUTOUT_NO_COLOR;
	}
	
	public static ShaderInstance getRendertypeWindow() {
		return RENDERTYPE_WINDOW;
	}
	
	public static void blitGUIUnscaled(GuiGraphics graphics, int tex, float x1, float y1, float x2, float y2) {
		float guiScale = (float) Minecraft.getInstance().getWindow().getGuiScale();
		x1 /= guiScale;
		y1 /= guiScale;
		x2 /= guiScale;
		y2 /= guiScale;
		
		blitGUI(graphics, tex, x1, y1, x2, y2, 0, 0, 1, 1);
	}
	
	public static void blitGUI(GuiGraphics graphics, int tex, float x1, float y1, float x2, float y2) {
		blitGUI(graphics, tex, x1, y1, x2, y2, 0, 0, 1, 1);
	}
	
	public static void blitGUI(GuiGraphics graphics, int tex, float x1, float y1, float x2, float y2, float u1, float v1, float u2, float v2) {
		RenderSystem.setShaderTexture(0, tex);
		RenderSystem.setShader(GameRenderer::getPositionTexShader);
		Matrix4f matrix4f = graphics.pose().last().pose();
		BufferBuilder bufferBuilder = Tesselator.getInstance().getBuilder();
		bufferBuilder.begin(VertexFormat.Mode.QUADS, DefaultVertexFormat.POSITION_TEX);
		bufferBuilder.vertex(matrix4f, x1, y1, 0).uv(u1, v1).endVertex();
		bufferBuilder.vertex(matrix4f, x1, y2, 0).uv(u1, v2).endVertex();
		bufferBuilder.vertex(matrix4f, x2, y2, 0).uv(u2, v2).endVertex();
		bufferBuilder.vertex(matrix4f, x2, y1, 0).uv(u2, v1).endVertex();
		BufferUploader.drawWithShader(bufferBuilder.end());
	}
	
	public static void blitGUI(GuiGraphics graphics, ResourceLocation tex, float x1, float y1, float x2, float y2) {
		blitGUI(graphics, tex, x1, y1, x2, y2, 0, 0, 1, 1);
	}
	
	public static void blitGUI(GuiGraphics graphics, ResourceLocation tex, float x1, float y1, float x2, float y2, float u1, float v1, float u2, float v2) {
		RenderSystem.setShaderTexture(0, tex);
		RenderSystem.setShader(GameRenderer::getPositionTexShader);
		Matrix4f matrix4f = graphics.pose().last().pose();
		BufferBuilder bufferBuilder = Tesselator.getInstance().getBuilder();
		bufferBuilder.begin(VertexFormat.Mode.QUADS, DefaultVertexFormat.POSITION_TEX);
		bufferBuilder.vertex(matrix4f, x1, y1, 0).uv(u1, v1).endVertex();
		bufferBuilder.vertex(matrix4f, x1, y2, 0).uv(u1, v2).endVertex();
		bufferBuilder.vertex(matrix4f, x2, y2, 0).uv(u2, v2).endVertex();
		bufferBuilder.vertex(matrix4f, x2, y1, 0).uv(u2, v1).endVertex();
		BufferUploader.drawWithShader(bufferBuilder.end());
	}
	
	public static Pose cameraTransformPose(Camera camera) {
		PoseStack matrixStack = new PoseStack();
		matrixStack.mulPose(Axis.XP.rotationDegrees(camera.getXRot()));
		matrixStack.mulPose(Axis.YP.rotationDegrees(camera.getYRot() + 180.0F));
		matrixStack.translate(-camera.getPosition().x, -camera.getPosition().y, -camera.getPosition().z);
		
		return matrixStack.last();
	}
	
}
