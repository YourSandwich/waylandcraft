package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import dev.evvie.waylandcraft.render.ShaderWindowPass;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.renderer.GameRenderer;

@Mixin(GameRenderer.class)
public class GameRendererMixin {

	// Iris composites inside GameRenderer.renderLevel; its return is the first
	// point the main framebuffer holds the final shaded image. ShaderWindowPass
	// draws the in-world windows here so a shaderpack cannot grade them.
	@Inject(method = "renderLevel", at = @At("RETURN"))
	public void renderLevel(DeltaTracker deltaTracker, CallbackInfo info) {
		ShaderWindowPass.drawPending();
	}

}
