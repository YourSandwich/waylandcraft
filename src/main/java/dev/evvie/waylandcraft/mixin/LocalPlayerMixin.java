package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import dev.evvie.waylandcraft.WaylandCraft;
import net.minecraft.client.player.LocalPlayer;

@Mixin(LocalPlayer.class)
public class LocalPlayerMixin {

	// Window placement uses Shift+scroll to move a window left/right; Shift is
	// also the sneak key. isShiftKeyDown is the single gate all sneak/crouch
	// behavior derives from, so force it false while placing to keep the avatar
	// upright. Movement is unaffected - it does not read this flag.
	@Inject(method = "isShiftKeyDown", at = @At("HEAD"), cancellable = true)
	public void waylandcraft$suppressSneakWhilePlacing(CallbackInfoReturnable<Boolean> cir) {
		if(WaylandCraft.instance != null && WaylandCraft.instance.isPlacingWindow()) cir.setReturnValue(false);
	}

}
