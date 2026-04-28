package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Invoker;

import com.mojang.blaze3d.pipeline.RenderPipeline;

import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.resources.ResourceLocation;

@Mixin(GuiGraphics.class)
public interface IGuiGraphics {
	
	@Invoker
	public void invokeInnerBlit(RenderPipeline pipeline, ResourceLocation location, int x1, int x2, int y1, int y2, float u1, float u2, float v1, float v2, int color);
	
}
