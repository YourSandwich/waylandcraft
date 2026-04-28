package dev.evvie.waylandcraft.render;

import java.util.function.Function;

import com.mojang.blaze3d.pipeline.BlendFunction;
import com.mojang.blaze3d.pipeline.RenderPipeline;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.PoseStack.Pose;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.blaze3d.vertex.VertexFormat;

import dev.evvie.waylandcraft.WaylandCraft;
import dev.evvie.waylandcraft.mixin.IGuiGraphics;
import net.minecraft.Util;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.renderer.MultiBufferSource.BufferSource;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.RenderStateShard;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

public class RenderUtils {
	
	private static final RenderPipeline.Snippet WINDOW_PIPELINE_SNIPPET = RenderPipeline.builder(RenderPipelines.MATRICES_PROJECTION_SNIPPET)
			.withVertexShader(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "core/rendertype_window"))
			.withFragmentShader(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "core/rendertype_window"))
			.withSampler("Sampler0")
			.withVertexFormat(DefaultVertexFormat.POSITION_TEX, VertexFormat.Mode.QUADS)
			.buildSnippet();
	
	private static final RenderPipeline WINDOW_CUTOUT_PIPELINE = RenderPipeline.builder(WINDOW_PIPELINE_SNIPPET)
			.withLocation(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_cutout"))
			.withShaderDefine("ALPHA_CUTOUT")
			.build();
	
	private static final RenderPipeline WINDOW_TRANSLUCENT_PIPELINE = RenderPipeline.builder(WINDOW_PIPELINE_SNIPPET)
			.withLocation(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_translucent"))
			.withBlend(BlendFunction.TRANSLUCENT)
			.build();
	
	private static final RenderPipeline WINDOW_CUTOUT_BACKGROUND_PIPELINE = RenderPipeline.builder(WINDOW_PIPELINE_SNIPPET)
			.withLocation(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_cutout_background"))
			.withShaderDefine("ALPHA_CUTOUT")
			.withShaderDefine("NO_COLOR")
			.build();
	
	private static final RenderPipeline WINDOW_TRANSLUCENT_BACKGROUND_PIPELINE = RenderPipeline.builder(WINDOW_PIPELINE_SNIPPET)
			.withLocation(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_translucent_background"))
			.withShaderDefine("NO_COLOR")
			.withBlend(BlendFunction.TRANSLUCENT)
			.build();
	
	public static final Function<ResourceLocation, RenderType> WINDOW_CUTOUT = Util.memoize(
		(location) -> {
			RenderType.CompositeState compositeState = RenderType.CompositeState.builder()
					.setTextureState(new RenderStateShard.TextureStateShard(location, false))
					.createCompositeState(false);
			return RenderType.create("window_cutout", RenderType.TRANSIENT_BUFFER_SIZE, false, true, WINDOW_CUTOUT_PIPELINE, compositeState);
		}
	);
	
	public static final Function<ResourceLocation, RenderType> WINDOW_TRANSLUCENT = Util.memoize(
		(location) -> {
			RenderType.CompositeState compositeState = RenderType.CompositeState.builder()
					.setTextureState(new RenderStateShard.TextureStateShard(location, false))
					.createCompositeState(false);
			return RenderType.create("window_translucent", RenderType.TRANSIENT_BUFFER_SIZE, false, true, WINDOW_TRANSLUCENT_PIPELINE, compositeState);
		}
	);
	
	public static final Function<ResourceLocation, RenderType> WINDOW_BACKGROUND_CUTOUT = Util.memoize(
		(location) -> {
			RenderType.CompositeState compositeState = RenderType.CompositeState.builder()
					.setTextureState(new RenderStateShard.TextureStateShard(location, false))
					.createCompositeState(false);
			return RenderType.create("window_cutout_background", RenderType.TRANSIENT_BUFFER_SIZE, false, true, WINDOW_CUTOUT_BACKGROUND_PIPELINE, compositeState);
		}
	);
	
	public static final Function<ResourceLocation, RenderType> WINDOW_BACKGROUND_TRANSLUCENT = Util.memoize(
		(location) -> {
			RenderType.CompositeState compositeState = RenderType.CompositeState.builder()
					.setTextureState(new RenderStateShard.TextureStateShard(location, false))
					.createCompositeState(false);
			return RenderType.create("window_translucent_background", RenderType.TRANSIENT_BUFFER_SIZE, false, true, WINDOW_TRANSLUCENT_BACKGROUND_PIPELINE, compositeState);
		}
	);
	
	public static final RenderPipeline WINDOW_BLIT = RenderPipeline.builder(RenderPipelines.MATRICES_PROJECTION_SNIPPET)
			.withLocation(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "pipeline/window_blit"))
			.withVertexShader(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "core/window_blit"))
			.withFragmentShader(ResourceLocation.fromNamespaceAndPath(WaylandCraft.MOD_ID, "core/window_blit"))
			.withSampler("Sampler0")
			.withBlend(BlendFunction.TRANSLUCENT)
			.withVertexFormat(DefaultVertexFormat.POSITION_TEX_COLOR, VertexFormat.Mode.QUADS)
			.build();
	
	public static void renderFramebuffer(WindowFramebuffer framebuffer, boolean cutout, Pose pose, Vec3 pos1, Vec3 pos2, Vec3 pos3, Vec3 pos4, Vec2 uv1, Vec2 uv2, Vec2 uv3, Vec2 uv4) {
		if(!framebuffer.isValid()) return;
		
		BufferSource source = Minecraft.getInstance().renderBuffers().bufferSource();
		
		// Front quad
		Function<ResourceLocation, RenderType> renderType = cutout ? WINDOW_CUTOUT : WINDOW_TRANSLUCENT;
		VertexConsumer buffer = source.getBuffer(renderType.apply(framebuffer.getTextureLocation()));
		buffer.addVertex(pose, pos1.toVector3f()).setUv(uv1.x, uv1.y);
		buffer.addVertex(pose, pos2.toVector3f()).setUv(uv2.x, uv2.y);
		buffer.addVertex(pose, pos3.toVector3f()).setUv(uv3.x, uv3.y);
		buffer.addVertex(pose, pos4.toVector3f()).setUv(uv4.x, uv4.y);
		
		// Back quad
		renderType = cutout ? WINDOW_BACKGROUND_CUTOUT : WINDOW_BACKGROUND_TRANSLUCENT;
		buffer = source.getBuffer(renderType.apply(framebuffer.getTextureLocation()));
		buffer.addVertex(pose, pos4.toVector3f()).setUv(uv4.x, uv4.y);
		buffer.addVertex(pose, pos3.toVector3f()).setUv(uv3.x, uv3.y);
		buffer.addVertex(pose, pos2.toVector3f()).setUv(uv2.x, uv2.y);
		buffer.addVertex(pose, pos1.toVector3f()).setUv(uv1.x, uv1.y);
	}
	
	public static void renderFramebuffer2D(GuiGraphics context, WindowFramebuffer framebuffer, int x, int y, int w, int h) {
		if(!framebuffer.isValid()) return;
		((IGuiGraphics) context).invokeInnerBlit(WINDOW_BLIT, framebuffer.getTextureLocation(), x, x + w, y, y + h, 0.0f, 1.0f, 0.0f, 1.0f, -1);
	}
	
	public static void cameraTransform(PoseStack poseStack, Camera camera) {
//		poseStack.mulPose(Axis.XP.rotationDegrees(camera.getXRot()));
//		poseStack.mulPose(Axis.YP.rotationDegrees(camera.getYRot() + 180.0F));
		poseStack.translate(-camera.getPosition().x, -camera.getPosition().y, -camera.getPosition().z);
	}
	
}
